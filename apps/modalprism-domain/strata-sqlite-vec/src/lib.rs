//! 把 `sqlite-vec` 注册成进程级 SQLite 自动扩展。
//!
//! `sqlx` 只支持按路径动态装载扩展，那样的启动要额外分发一个动态库；
//! 这里改为静态链接：`sqlite-vec` 的 C 源码直接编进二进制，注册一次后
//! 由 SQLite 在每个新连接上自动回调 `sqlite3_vec_init`，因此
//! `vec_version()`、`vec_distance_cosine()` 在连接池里天然可用。
//!
//! 注册是一次性的：`OnceLock` 缓存第一次的结果，重复调用返回同一结论。
//!
//! # 为什么这个 crate 允许 unsafe
//!
//! `sqlite3_auto_extension` 的参数是 C 函数指针，而 `sqlite-vec` 把
//! `sqlite3_vec_init` 声明成无参签名，两者只能靠 `std::mem::transmute`
//! 对齐。这段 `unsafe` 收敛在本 crate 的 `register` 里，调用方拿到的是
//! 普通 `Result`。

use std::sync::OnceLock;

/// `sqlite3_auto_extension` 期望的入口签名。
type VecEntry = unsafe extern "C" fn(
    *mut libsqlite3_sys::sqlite3,
    *mut *mut std::os::raw::c_char,
    *const libsqlite3_sys::sqlite3_api_routines,
) -> std::os::raw::c_int;

static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();

/// 注册 `sqlite-vec`。每个进程只需成功一次。
///
/// 必须在打开任何 SQLite 连接之前调用；已打开的连接不会补装扩展。
/// 重复调用返回第一次的结果，不会重复注册。
pub fn register() -> Result<(), String> {
    REGISTERED
        .get_or_init(|| {
            // SAFETY: `sqlite3_vec_init` 是 sqlite-vec 用 `SQLITE_CORE` 编译出的
            // 自动扩展入口，签名与 `VecEntry` 的调用约定一致（C ABI、无参数值）。
            // 该符号与最终链接进来的 libsqlite3-sys 是同一份 sqlite3。
            let entry: VecEntry = unsafe {
                std::mem::transmute::<unsafe extern "C" fn(), VecEntry>(
                    sqlite_vec::sqlite3_vec_init,
                )
            };
            // SAFETY: 传的是上面校验过签名的 C 函数指针，SQLite 只会保存它。
            let code = unsafe { libsqlite3_sys::sqlite3_auto_extension(Some(entry)) };
            if code == 0 {
                Ok(())
            } else {
                Err(format!("sqlite3_auto_extension 返回 {code}"))
            }
        })
        .clone()
}

/// 本次进程是否已成功注册。
pub fn registered() -> bool {
    matches!(REGISTERED.get(), Some(Ok(())))
}
