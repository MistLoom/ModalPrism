//! 作业事件（第一期第一步）。
//!
//! 浏览器取数与详情跟页沿用现有 planner 的租约、截止、重试与幂等执行，
//! 这里只把执行进度以事件形式广播出去，不另造工作流引擎：
//! - `raw.ready`：访问器产出信封（原文就绪）
//! - `normalized.ready`：管道产出数据面（规整就绪）
//! - `job.failed`：任务按错误分类失败
//! - `job.cancelled`：任务被取消（超时 / 显式取消）

use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};
use strata_model::AccessTask;

pub type EventSink = Arc<dyn Fn(&str, &Value) + Send + Sync>;

/// 进程内事件出口。未注册 sink 时事件为空操作。
#[derive(Clone, Default)]
pub struct EventBus {
    sink: Arc<OnceLock<EventSink>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册事件出口；只有第一次注册生效（与 parse 钩子同款单写语义）。
    pub fn set_sink(&self, sink: EventSink) -> bool {
        self.sink.set(sink).is_ok()
    }

    pub fn emit(&self, event: &str, payload: &Value) {
        if let Some(sink) = self.sink.get() {
            sink(event, payload);
        }
    }

    /// 原文就绪：访问器（含浏览器访问器）产出信封之后、管道之前发出。
    pub fn raw_ready(&self, task: &AccessTask, envelopes: usize) {
        self.emit(
            "raw.ready",
            &json!({
                "taskId": task.task_id.as_str(),
                "sourceId": task.source_id.as_str(),
                "planId": task.plan_id,
                "accessor": task.accessor,
                "reason": task.reason,
                "envelopes": envelopes,
            }),
        );
    }

    /// 规整就绪：管道产出数据面之后发出。
    pub fn normalized_ready(&self, task: &AccessTask, ops: usize) {
        self.emit(
            "normalized.ready",
            &json!({
                "taskId": task.task_id.as_str(),
                "sourceId": task.source_id.as_str(),
                "planId": task.plan_id,
                "accessor": task.accessor,
                "reason": task.reason,
                "ops": ops,
            }),
        );
    }

    /// 处理就绪：内容处理器在数据面上追加派生片段之后发出。
    /// 只在计划/任务（含 once）管道声明了 content-processor 挂载时发出；
    /// 未挂载不跑、不发，不进入默认采集热路径。
    pub fn processed_ready(&self, task: &AccessTask, derived: usize) {
        self.emit(
            "processed.ready",
            &json!({
                "taskId": task.task_id.as_str(),
                "sourceId": task.source_id.as_str(),
                "planId": task.plan_id,
                "accessor": task.accessor,
                "reason": task.reason,
                "derived": derived,
            }),
        );
    }

    /// 因子就绪：因子（测度）通过在数据面上追加测度记录之后发出。
    /// 只在管道显式声明了 factor/ 挂载时发出；未挂载不跑、不发。
    pub fn factor_ready(&self, task: &AccessTask, factors: usize) {
        self.emit(
            "factor.ready",
            &json!({
                "taskId": task.task_id.as_str(),
                "sourceId": task.source_id.as_str(),
                "planId": task.plan_id,
                "accessor": task.accessor,
                "reason": task.reason,
                "factors": factors,
            }),
        );
    }

    /// 向量写入就绪：嵌入派生任务成功把向量写进索引之后发出。
    /// 只在 vectorize 阶段被显式触发且本轮确有嵌入产出时发出；未开启不跑、不发。
    pub fn vectorize_ready(&self, vectors: usize) {
        self.emit(
            "vectorize.ready",
            &json!({
                "vectors": vectors,
            }),
        );
    }

    /// 任务失败：带错误分类与消息。
    pub fn job_failed(&self, task: &AccessTask, class: &str, message: &str) {
        self.emit(
            "job.failed",
            &json!({
                "taskId": task.task_id.as_str(),
                "sourceId": task.source_id.as_str(),
                "planId": task.plan_id,
                "class": class,
                "message": message,
            }),
        );
    }

    /// 任务取消：超时或显式取消。
    pub fn job_cancelled(&self, task: &AccessTask, message: &str) {
        self.emit(
            "job.cancelled",
            &json!({
                "taskId": task.task_id.as_str(),
                "sourceId": task.source_id.as_str(),
                "planId": task.plan_id,
                "message": message,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;
    use std::sync::Mutex;

    #[test]
    fn bus_broadcasts_ready_failed_and_cancelled_events() {
        let bus = EventBus::new();
        let seen: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = Arc::clone(&seen);
        assert!(bus.set_sink(Arc::new(move |event, payload| {
            sink_seen.lock().unwrap().push((event.to_string(), payload.clone()));
        })));

        let task = sample_task();
        bus.raw_ready(&task, 2);
        bus.normalized_ready(&task, 5);
        bus.processed_ready(&task, 3);
        bus.factor_ready(&task, 2);
        bus.job_failed(&task, "config", "未知组件");
        bus.job_cancelled(&task, "超时");

        let events = seen.lock().unwrap();
        let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "raw.ready",
                "normalized.ready",
                "processed.ready",
                "factor.ready",
                "job.failed",
                "job.cancelled"
            ]
        );
        assert_eq!(events[0].1["taskId"], "t_1");
        assert_eq!(events[0].1["envelopes"], 2);
        assert_eq!(events[1].1["ops"], 5);
        assert_eq!(events[2].1["derived"], 3);
        assert_eq!(events[3].1["factors"], 2);
        assert_eq!(events[4].1["class"], "config");
    }

    #[test]
    fn bus_without_sink_is_a_noop() {
        let bus = EventBus::new();
        let task = sample_task();
        bus.raw_ready(&task, 1);
        bus.job_failed(&task, "transient", "网络");
    }

    fn sample_task() -> AccessTask {
        AccessTask {
            task_id: strata_model::TaskId::new("t_1").unwrap(),
            source_id: strata_model::SourceId::new("source/demo").unwrap(),
            plan_id: Some("plan/demo".into()),
            accessor: "accessor/browser".into(),
            locator: strata_model::Locator::Url {
                url: "https://example.com/a".into(),
            },
            spec: Value::Object(Map::new()),
            reason: strata_model::TaskReason::Follow,
            priority: 20,
            queue: strata_model::QueueName::Free,
            lease_until: strata_model::Timestamp::now(),
            attempt: 0,
            batch_key: None,
            pipeline: None,
            vectorize: false,
            generation: 1,
        }
    }
}
