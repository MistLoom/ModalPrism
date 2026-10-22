//! 反馈层：支撑层内的顺序记录。不是 EventBus，不是 Ability，不依赖 frame 以外。
//!
//! 能力作者只用 [`feedback!`]。查询 [`list`] / [`get`] / [`purge`]。
//! 处理器 [`register_processor`]。引擎与客端装配见 [`host`]。

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

/// 无调用上下文时的链路预设。
pub const DETACHED_CHAIN: &str = "detached";

const TAG_MAX: usize = 19;

/// 编译期架构位置（crate 包名 + 模块路径）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchLoc {
    pub ability: String,
    pub module: String,
}

/// 运行期链路范围。由引擎写入 task-local；无范围时用 [`FeedbackScope::detached`]。
#[derive(Clone, Debug)]
pub struct FeedbackScope {
    pub chain_id: String,
    pub span: u64,
}

impl FeedbackScope {
    pub fn detached() -> Self {
        Self {
            chain_id: DETACHED_CHAIN.into(),
            span: 0,
        }
    }
}

/// 一条反馈记录（亦为日志条目）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackRecord {
    pub id: String,
    pub at: u64,
    pub loc: ArchLoc,
    pub tags: Vec<String>,
    pub desc: String,
    pub chain_id: String,
    pub span: u64,
}

/// 消费端标签过滤。空列表表示接收全部。匹配：记录含任一指定标签。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TagFilter {
    pub tags: Vec<String>,
}

impl TagFilter {
    pub fn all() -> Self {
        Self { tags: Vec::new() }
    }

    pub fn only(tags: Vec<String>) -> Self {
        Self { tags }
    }

    pub fn matches(&self, rec: &FeedbackRecord) -> bool {
        if self.tags.is_empty() {
            return true;
        }
        rec.tags.iter().any(|t| self.tags.iter().any(|w| w == t))
    }
}

/// 反馈处理器。按登记顺序同步调用；失败不影响投递与后续处理器。
pub trait FeedbackProcessor: Send + Sync {
    fn on_feedback(&self, record: &FeedbackRecord);
}

struct State {
    path: PathBuf,
    records: Vec<FeedbackRecord>,
    processors: Vec<(u64, Arc<dyn FeedbackProcessor>, TagFilter)>,
    next_proc: u64,
}

fn default_path() -> PathBuf {
    std::env::current_dir()
        .expect("反馈落盘需要当前工作目录")
        .join("data")
        .join("feedback")
        .join("events.jsonl")
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        let path = default_path();
        Mutex::new(State {
            records: load_file(&path),
            path,
            processors: Vec::new(),
            next_proc: 1,
        })
    })
}

static SCOPE: OnceLock<fn() -> FeedbackScope> = OnceLock::new();
static FORWARD: RwLock<Option<Arc<dyn Fn(FeedbackRecord) + Send + Sync>>> = RwLock::new(None);

fn current_scope() -> FeedbackScope {
    SCOPE
        .get()
        .map(|f| f())
        .unwrap_or_else(FeedbackScope::detached)
}

/// 供 [`feedback!`] 展开；能力代码不要直接调用。
#[doc(hidden)]
pub const fn assert_tag(tag: &str) {
    assert!(
        !tag.is_empty() && tag.len() <= TAG_MAX,
        "标签须为 1..19 英文字母"
    );
    let b = tag.as_bytes();
    let mut i = 0;
    while i < b.len() {
        assert!(b[i].is_ascii_alphabetic(), "标签只允许英文字母");
        i += 1;
    }
}

fn valid_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= TAG_MAX && tag.bytes().all(|c| c.is_ascii_alphabetic())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn load_file(path: &Path) -> Vec<FeedbackRecord> {
    let Ok(f) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (i, line) in BufReader::new(f).lines().enumerate() {
        let line = line.unwrap_or_else(|e| panic!("读反馈落盘失败 {}: {e}", path.display()));
        if line.is_empty() {
            continue;
        }
        out.push(
            serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("反馈落盘损坏 {} 行 {}: {e}", path.display(), i + 1)),
        );
    }
    out
}

fn append_line(path: &Path, rec: &FeedbackRecord) {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .unwrap_or_else(|e| panic!("创建反馈目录失败 {}: {e}", dir.display()));
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap_or_else(|e| panic!("打开反馈落盘失败 {}: {e}", path.display()));
    let line = serde_json::to_string(rec).expect("FeedbackRecord 可序列化");
    writeln!(f, "{line}").unwrap_or_else(|e| panic!("写入反馈落盘失败 {}: {e}", path.display()));
    f.flush()
        .unwrap_or_else(|e| panic!("刷新反馈落盘失败 {}: {e}", path.display()));
}

fn persist(rec: &FeedbackRecord) -> Vec<Arc<dyn FeedbackProcessor>> {
    let mut s = state().lock().expect("feedback");
    append_line(&s.path, rec);
    s.records.push(rec.clone());
    s.processors
        .iter()
        .filter(|(_, _, f)| f.matches(rec))
        .map(|(_, p, _)| Arc::clone(p))
        .collect()
}

fn notify(procs: Vec<Arc<dyn FeedbackProcessor>>, rec: &FeedbackRecord) {
    for p in procs {
        let rec = rec.clone();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            p.on_feedback(&rec);
        }));
    }
}

fn commit(rec: FeedbackRecord) {
    let remote = FORWARD.read().expect("feedback forward").clone();
    let procs = persist(&rec);
    if let Some(f) = remote {
        f(rec.clone());
    }
    notify(procs, &rec);
}

/// 供 [`feedback!`] 展开；能力代码不要直接调用。
#[doc(hidden)]
pub fn submit(ability: &'static str, module: &'static str, tags: &[&str], desc: String) {
    let tags: Vec<String> = tags
        .iter()
        .copied()
        .filter(|t| valid_tag(t))
        .map(str::to_string)
        .collect();
    let scope = current_scope();
    commit(FeedbackRecord {
        id: uuid::Uuid::new_v4().to_string(),
        at: now_ms(),
        loc: ArchLoc {
            ability: ability.into(),
            module: module.into(),
        },
        tags,
        desc,
        chain_id: scope.chain_id,
        span: scope.span,
    });
}

/// 登记处理器。丢弃返回值即卸掉该次登记。
pub struct ProcessorReg {
    id: u64,
}

impl Drop for ProcessorReg {
    fn drop(&mut self) {
        if let Ok(mut s) = state().lock() {
            s.processors.retain(|(id, _, _)| *id != self.id);
        }
    }
}

pub fn register_processor(
    processor: Arc<dyn FeedbackProcessor>,
    filter: TagFilter,
) -> ProcessorReg {
    let mut s = state().lock().expect("feedback");
    let id = s.next_proc;
    s.next_proc += 1;
    s.processors.push((id, processor, filter));
    ProcessorReg { id }
}

/// 按写入顺序返回全部记录。标签 / 链路由调用方筛选。
pub fn list() -> Vec<FeedbackRecord> {
    state().lock().expect("feedback").records.clone()
}

pub fn get(id: &str) -> Option<FeedbackRecord> {
    state()
        .lock()
        .expect("feedback")
        .records
        .iter()
        .find(|r| r.id == id)
        .cloned()
}

pub fn purge() {
    let mut s = state().lock().expect("feedback");
    s.records.clear();
    if s.path.exists() {
        fs::write(&s.path, [])
            .unwrap_or_else(|e| panic!("清空反馈落盘失败 {}: {e}", s.path.display()));
    }
}

/// 引擎与客端装配。能力作者不要使用。
pub mod host {
    use super::*;

    /// 绑定落盘路径并载入已有记录。缺省 `{cwd}/data/feedback/events.jsonl`。
    pub fn bind(path: impl AsRef<Path>) {
        let path = path.as_ref().to_path_buf();
        let recs = load_file(&path);
        let mut s = state().lock().expect("feedback");
        s.path = path;
        s.records = recs;
    }

    /// 宿主收下客端记录。只落盘并通知处理器，不再次转发。
    pub fn accept(record: FeedbackRecord) {
        let procs = persist(&record);
        notify(procs, &record);
    }

    pub fn install_scope(getter: fn() -> FeedbackScope) {
        let _ = SCOPE.set(getter);
    }

    /// 客端把记录转给宿主；本进程仍顺序落盘并通知本地处理器。
    pub fn forward(f: Arc<dyn Fn(FeedbackRecord) + Send + Sync>) {
        *FORWARD.write().expect("feedback forward") = Some(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_rules() {
        assert!(valid_tag("warn"));
        assert!(!valid_tag(""));
        assert!(!valid_tag("a_b"));
        const _: () = assert_tag("ok");
    }
}
