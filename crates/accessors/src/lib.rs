//! 访问器契约。
//!
//! 访问器经端口取回字节并输出原始信封。适配器写入和出站治理不在本 crate。

mod error;
mod redact;
mod types;

pub use error::AccessError;
pub use redact::{redact_envelope, redact_headers, redact_names};
pub use types::{
    AccessContext, AccessHints, AccessRequest, Accessor, AccessorCapabilities, CaptureNetwork,
    EgressGateway, HttpRequest, HttpResponse, PaidCall, PaidCapability, PaidGateway, PaidResponse,
    Session, SessionPool, Staging, config_error,
};
