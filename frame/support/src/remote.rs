//! 远程 Ability 信封。子进程 / WASM 只是本协议的传输。
//!
//! 一行一帧 JSON。能力实现仍只见 `Ability` / `AbilityCtx`。

/// 线协议版本（与 `framework_version` 独立）。
pub const PROTOCOL_VERSION: &str = "1";

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AbilityManifest, BusEvent, Error, ErrorKind, EventFilter, EventMode, FeedbackRecord, Lifecycle,
    LoadSpec, Result,
};

/// 一帧。`reply_to` 指向被回复的请求 id。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub body: Body,
}

impl Frame {
    pub fn request(body: Body) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            reply_to: None,
            body,
        }
    }

    pub fn reply(to: &str, body: Body) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            reply_to: Some(to.to_string()),
            body,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Body {
    Hello,
    Manifest {
        manifest: AbilityManifest,
    },
    Create {
        config: Value,
    },
    Lifecycle {
        phase: Lifecycle,
        instance: String,
        chain_id: String,
        span: u64,
    },
    Invoke {
        method: String,
        args: Value,
        instance: String,
        chain_id: String,
        span: u64,
    },
    Ok {
        #[serde(default)]
        value: Value,
    },
    Err {
        error: WireError,
    },
    /// 客端回呼宿主 `AbilityCtx`。`ctx_id` 对应宿主登记的句柄。
    Ctx {
        ctx_id: String,
        op: CtxOp,
    },
    Event {
        sub: String,
        event: BusEvent,
    },
    /// 宿主投递已登记的客端处理者。
    RunHandle {
        guest_id: String,
        event: BusEvent,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_token: Option<String>,
    },
    Shutdown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CtxOp {
    Call {
        id_or_alias: String,
        method: String,
        args: Value,
    },
    Dep {
        id_or_alias: String,
    },
    Catalog,
    LocalGet {
        key: String,
    },
    LocalSet {
        key: String,
        value: Value,
    },
    Emit {
        name: String,
        payload: Value,
    },
    Load {
        spec: LoadSpec,
    },
    Unload {
        instance: String,
    },
    Subscribe {
        filter: EventFilter,
    },
    Handle {
        filter: EventFilter,
        prepend: bool,
        mode: EventMode,
        guest_id: String,
    },
    Unhandle {
        id: u64,
    },
    EventNext {
        token: String,
        event: BusEvent,
    },
    Share,
    Feedback {
        record: FeedbackRecord,
    },
    ChainStatus {
        #[serde(default)]
        chain_id: Option<String>,
    },
}

/// 跨进程错误。`kind` 为 [`ErrorKind::code`]。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WireError {
    pub kind: String,
    pub message: String,
}

impl From<&Error> for WireError {
    fn from(e: &Error) -> Self {
        Self {
            kind: e.kind().code().to_string(),
            message: e.to_string(),
        }
    }
}

impl WireError {
    pub fn into_error(self) -> Error {
        let kind = ErrorKind::from_code(&self.kind).unwrap_or(ErrorKind::Failed);
        Error::new(kind, self.message)
    }
}

pub fn encode_frame(frame: &Frame) -> Result<String> {
    let mut line = serde_json::to_string(frame).map_err(|e| Error::failed(e.to_string()))?;
    line.push('\n');
    Ok(line)
}

pub fn decode_frame(line: &str) -> Result<Frame> {
    let line = line.trim_end_matches(['\r', '\n']);
    serde_json::from_str(line).map_err(|e| Error::failed(format!("远程帧无法解码: {e}")))
}

pub fn ok_value(frame: Frame) -> Result<Value> {
    match frame.body {
        Body::Ok { value } => Ok(value),
        Body::Err { error } => Err(error.into_error()),
        other => Err(Error::failed(format!("远程应答类型不符: {other:?}"))),
    }
}
