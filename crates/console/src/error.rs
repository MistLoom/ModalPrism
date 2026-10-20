use serde_json::json;

#[derive(Debug)]
pub struct ConsoleError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl ConsoleError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            code: "bad_request",
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: 404,
            code: "not_found",
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            status: 500,
            code: "internal",
            message: sanitize_internal_message(message.into()),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: 403,
            code: "forbidden",
            message: message.into(),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        })
    }
}

pub type ConsoleResult<T> = Result<T, ConsoleError>;

fn sanitize_internal_message(message: String) -> String {
    if message.contains("spec 无效") || message.contains("spec") {
        return "无法读取该记录，请稍后重试。".into();
    }
    message
}
