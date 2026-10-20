//! 内置访问器。对外请求只经过出站或付费端口。

mod api;
mod browser;
mod envelope;
mod file;
mod headers;
mod http;
mod outbound;
mod paid;
mod push;
mod ready;
mod rss;
mod search;
mod websub;

pub use api::ApiAccessor;
pub use browser::{
    BrowserAccessor, BrowserEngine, EngineOutput, EngineRequest, MatchedBody, MoliCommand,
    moli_args, parse_dump,
};
pub use file::FileAccessor;
pub use http::HttpAccessor;
pub use paid::PaidApiAccessor;
pub use push::{Incoming, PushAccessor};
pub use rss::RssAccessor;
pub use search::SearchAccessor;
pub use websub::{Challenge, Notice, notify, subscribe, verify};

use accessors::Accessor;

pub fn by_id(id: &str) -> Option<Box<dyn Accessor>> {
    match id {
        "http" => Some(Box::new(HttpAccessor)),
        "rss" => Some(Box::new(RssAccessor)),
        "api" => Some(Box::new(ApiAccessor)),
        "paid-api" => Some(Box::new(PaidApiAccessor)),
        "search" => Some(Box::new(SearchAccessor)),
        "push" => Some(Box::new(PushAccessor::new())),
        "file" => Some(Box::new(FileAccessor)),
        _ => None,
    }
}
