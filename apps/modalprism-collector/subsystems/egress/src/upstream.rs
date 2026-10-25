//! 上游代理设置。默认跟随系统，命中时由代理解析目标主机。
//!
//! `system` 依次看环境变量与 Windows 的 Internet Settings。代理生效时本地不做
//! DNS 固定与私网拦截：目标地址由代理一侧解析，本地只保留 scheme 与凭据检查。

use std::env;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxySetting {
    System,
    Off,
    Manual(String),
}

impl ProxySetting {
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None | Some("system") => Self::System,
            Some("off") => Self::Off,
            Some(url) => Self::Manual(url.to_string()),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Off => "off",
            Self::Manual(_) => "manual",
        }
    }

    pub fn manual(&self) -> Option<&str> {
        match self {
            Self::Manual(url) => Some(url),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProxyRoute {
    /// 含可能的认证信息，只交给传输层。
    pub url: String,
    /// `env:<NAME>`、`windows` 或 `config`。
    pub source: String,
}

impl ProxyRoute {
    /// 去掉用户名与密码，供状态接口和日志使用。
    pub fn display(&self) -> String {
        let Ok(mut url) = Url::parse(&self.url) else {
            return "<invalid>".into();
        };
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.to_string()
    }
}

/// 目标地址是否应交给上游代理。
pub fn route(setting: &ProxySetting, target: &Url) -> Option<ProxyRoute> {
    match setting {
        ProxySetting::Off => None,
        ProxySetting::Manual(url) => {
            let no_proxy = no_proxy_env();
            if bypassed(target, &no_proxy) {
                return None;
            }
            Some(ProxyRoute {
                url: normalize(url),
                source: "config".into(),
            })
        }
        ProxySetting::System => system_route(target),
    }
}

/// 从配置读取代理设置。
///
/// 接受两种形状：带 `policy` 包裹的系统配置，以及已经叠加过的策略对象本身。
/// 代理只有一个声明位置（`egress.proxy`），采集、加工与通知都从这里取值，
/// 避免各自解析字符串而出现「界面改了、某条链路没生效」。
pub fn setting_of(config: &Value) -> ProxySetting {
    let raw = config
        .pointer("/policy/egress/proxy")
        .or_else(|| config.pointer("/egress/proxy"))
        .and_then(Value::as_str);
    ProxySetting::parse(raw)
}

/// 探测 https 目标的上游代理，供启动摘要与状态接口展示。
pub fn probe(setting: &ProxySetting) -> Option<ProxyRoute> {
    let target = Url::parse("https://example.com/").ok()?;
    route(setting, &target)
}

fn system_route(target: &Url) -> Option<ProxyRoute> {
    let no_proxy = no_proxy_env();
    if bypassed(target, &no_proxy) {
        return None;
    }
    if let Some((value, name)) = env_proxy(target.scheme()) {
        return Some(ProxyRoute {
            url: normalize(&value),
            source: format!("env:{name}"),
        });
    }
    let (server, overrides) = registry_proxy()?;
    if bypassed(target, &overrides) {
        return None;
    }
    let server = pick_server(target.scheme(), &server)?;
    Some(ProxyRoute {
        url: normalize(&server),
        source: "windows".into(),
    })
}

fn env_proxy(scheme: &str) -> Option<(String, &'static str)> {
    let (preferred, fallback) = if scheme == "https" {
        (["HTTPS_PROXY", "https_proxy"], ["ALL_PROXY", "all_proxy"])
    } else {
        (["HTTP_PROXY", "http_proxy"], ["ALL_PROXY", "all_proxy"])
    };
    for name in preferred {
        if let Some(value) = env_value(name) {
            return Some((value, name));
        }
    }
    for name in fallback {
        if let Some(value) = env_value(name) {
            return Some((value, name));
        }
    }
    None
}

fn env_value(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn no_proxy_env() -> String {
    ["NO_PROXY", "no_proxy"]
        .iter()
        .find_map(|name| env_value(name))
        .unwrap_or_default()
}

#[cfg(windows)]
fn registry_proxy() -> Option<(String, String)> {
    use windows_registry::CURRENT_USER;

    let key = CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    if key.get_u32("ProxyEnable").ok().unwrap_or(0) == 0 {
        return None;
    }
    let server = key.get_string("ProxyServer").ok()?;
    if server.trim().is_empty() {
        return None;
    }
    let overrides = key.get_string("ProxyOverride").unwrap_or_default();
    Some((server, overrides))
}

#[cfg(not(windows))]
fn registry_proxy() -> Option<(String, String)> {
    None
}

/// `ProxyServer` 可以是 `host:port`，也可以是 `http=host:port;https=host:port`。
fn pick_server(scheme: &str, server: &str) -> Option<String> {
    let text = server.trim();
    if !text.contains('=') {
        return (!text.is_empty()).then(|| text.to_string());
    }
    let mut http = None;
    for item in text.split(';') {
        let Some((key, value)) = item.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if key.eq_ignore_ascii_case(scheme) {
            return Some(value.to_string());
        }
        if key.eq_ignore_ascii_case("http") {
            http = Some(value.to_string());
        }
    }
    http
}

fn normalize(value: &str) -> String {
    let text = value.trim();
    if text.contains("://") {
        text.to_string()
    } else {
        format!("http://{text}")
    }
}

/// `NO_PROXY` 使用逗号分隔，Windows 的 `ProxyOverride` 使用分号。
pub fn bypassed(target: &Url, list: &str) -> bool {
    let Some(host) = target.host_str() else {
        return false;
    };
    if host.eq_ignore_ascii_case("localhost") || is_loopback(host) {
        return true;
    }
    let port = target.port_or_known_default();
    for raw in list.split([',', ';']) {
        let entry = raw.trim().to_ascii_lowercase();
        if entry.is_empty() {
            continue;
        }
        if entry == "*" {
            return true;
        }
        if entry == "<local>" {
            if !host.contains('.') {
                return true;
            }
            continue;
        }
        let entry = entry.trim_start_matches("*.");
        let entry = entry.trim_start_matches('.');
        let (entry_host, entry_port) = split_port(entry);
        if let Some(entry_port) = entry_port {
            if Some(entry_port) != port {
                continue;
            }
        }
        if host.eq_ignore_ascii_case(entry_host)
            || host
                .to_ascii_lowercase()
                .ends_with(&format!(".{entry_host}"))
        {
            return true;
        }
    }
    false
}

fn split_port(entry: &str) -> (&str, Option<u16>) {
    if entry.starts_with('[') {
        if let Some(end) = entry.find(']') {
            let host = &entry[..=end];
            let port = entry[end + 1..]
                .strip_prefix(':')
                .and_then(|value| value.parse().ok());
            return (host, port);
        }
    }
    match entry.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => match port.parse() {
            Ok(port) => (host, Some(port)),
            Err(_) => (entry, None),
        },
        _ => (entry, None),
    }
}

fn is_loopback(host: &str) -> bool {
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_setting() {
        assert_eq!(ProxySetting::parse(None), ProxySetting::System);
        assert_eq!(ProxySetting::parse(Some("off")), ProxySetting::Off);
        assert_eq!(
            ProxySetting::parse(Some(" http://127.0.0.1:7890 ")),
            ProxySetting::Manual("http://127.0.0.1:7890".into())
        );
    }

    #[test]
    fn reads_setting_from_system_policy() {
        let system = serde_json::json!({"policy": {"egress": {"proxy": "off"}}});
        assert_eq!(setting_of(&system), ProxySetting::Off);
        let system = serde_json::json!({"policy": {"egress": {"proxy": "127.0.0.1:7890"}}});
        assert_eq!(
            setting_of(&system),
            ProxySetting::Manual("127.0.0.1:7890".into())
        );
        assert_eq!(setting_of(&serde_json::json!({})), ProxySetting::System);
        let policy = serde_json::json!({"egress": {"proxy": "127.0.0.1:7891"}});
        assert_eq!(
            setting_of(&policy),
            ProxySetting::Manual("127.0.0.1:7891".into())
        );
    }

    #[test]
    fn picks_scheme_server() {
        let server = "http=127.0.0.1:7890;https=127.0.0.1:7891;socks=127.0.0.1:7892";
        assert_eq!(
            pick_server("https", server).as_deref(),
            Some("127.0.0.1:7891")
        );
        assert_eq!(
            pick_server("http", server).as_deref(),
            Some("127.0.0.1:7890")
        );
        assert_eq!(
            pick_server("ftp", server).as_deref(),
            Some("127.0.0.1:7890")
        );
        assert_eq!(
            pick_server("https", "proxy:8080").as_deref(),
            Some("proxy:8080")
        );
    }

    #[test]
    fn bypass_rules() {
        let url = Url::parse("https://api.example.com/x").unwrap();
        assert!(bypassed(&url, "example.com"));
        assert!(bypassed(&url, ".example.com"));
        assert!(bypassed(&url, "*.example.com"));
        assert!(bypassed(&url, "other.com, example.com:443"));
        assert!(!bypassed(&url, "other.com"));
        assert!(!bypassed(&url, "example.com:8443"));
        let local = Url::parse("http://intranet/").unwrap();
        assert!(bypassed(&local, "<local>"));
        assert!(bypassed(&Url::parse("http://localhost:8080/").unwrap(), ""));
    }

    #[test]
    fn manual_route_respects_no_proxy() {
        let setting = ProxySetting::Manual("127.0.0.1:7890".into());
        let url = Url::parse("https://example.com/").unwrap();
        assert_eq!(
            route(&setting, &url).map(|item| item.url),
            Some("http://127.0.0.1:7890".into())
        );
        let off = route(&ProxySetting::Off, &url);
        assert!(off.is_none());
    }

    #[test]
    fn redacts_credentials() {
        let route = ProxyRoute {
            url: "http://user:secret@127.0.0.1:7890".into(),
            source: "config".into(),
        };
        assert_eq!(route.display(), "http://127.0.0.1:7890/");
    }
}
