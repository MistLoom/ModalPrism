//! 编译期 Provider 注册表（build.rs 生成）。

include!(concat!(env!("OUT_DIR"), "/providers.rs"));
