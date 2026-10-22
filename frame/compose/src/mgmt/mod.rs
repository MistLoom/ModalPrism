//! 管理面 API：Host 原语，不含 admin 业务 UI。

mod routes;

pub use routes::{MgmtState, router};
