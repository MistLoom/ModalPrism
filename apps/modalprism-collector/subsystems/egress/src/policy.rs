//! 一次请求使用的出站策略。字段来自有效配置，由调用方填入。

use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct RateLimit {
    pub per_sec_micros: i64,
    pub burst: u32,
    pub concurrency: u32,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            per_sec_micros: 1_000_000,
            burst: 3,
            concurrency: 2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateNetworks {
    Deny,
    Allow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RobotsMode {
    Obey,
    ObeyWithExemptions,
    Ignore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unreachable {
    Rfc9309,
    Allow,
    Disallow,
}

#[derive(Clone, Copy, Debug)]
pub enum CrawlDelayMode {
    Honor,
    Ignore,
    Cap(Duration),
}

#[derive(Clone, Debug)]
pub struct Exempt {
    pub category: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct Policy {
    pub private_networks: PrivateNetworks,
    pub user_agent: String,
    pub contact: Option<String>,
    pub robots: RobotsMode,
    pub robots_ttl: Duration,
    pub on_unreachable: Unreachable,
    pub crawl_delay: CrawlDelayMode,
    pub exempt: Option<Exempt>,
    pub rate: RateLimit,
    pub host_rates: BTreeMap<String, RateLimit>,
    pub check_robots: bool,
}

impl Policy {
    pub fn library(user_agent: impl Into<String>) -> Self {
        Self {
            private_networks: PrivateNetworks::Deny,
            user_agent: user_agent.into(),
            contact: None,
            robots: RobotsMode::Obey,
            robots_ttl: Duration::from_secs(24 * 60 * 60),
            on_unreachable: Unreachable::Rfc9309,
            crawl_delay: CrawlDelayMode::Ignore,
            exempt: None,
            rate: RateLimit::default(),
            host_rates: BTreeMap::new(),
            check_robots: true,
        }
    }
}
