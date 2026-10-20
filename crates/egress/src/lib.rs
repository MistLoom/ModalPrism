//! 出站网关。库调用与正向代理共用 SSRF、限速、熔断和 robots。

mod domain;
mod fail;
mod gateway;
mod persist;
mod policy;
mod proxy;
mod robots;
mod ssrf;
mod state;

pub use domain::registrable;
pub use gateway::{Dial, Gateway, Resolver, TcpDial, Transport};
pub use persist::{MemoryPersist, Persist};
pub use policy::{
    CrawlDelayMode, Exempt, Policy, PrivateNetworks, RateLimit, RobotsMode, Unreachable,
};
pub use proxy::{serve, serve_each};
pub use ssrf::is_blocked;
