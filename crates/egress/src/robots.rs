//! robots.txt 的分组与路径规则。

use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub allow: bool,
    pub pattern: String,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub agents: Vec<String>,
    pub rules: Vec<Rule>,
    pub crawl_delay: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Disallow,
}

pub fn parse(body: &str) -> Vec<Group> {
    let mut groups = Vec::new();
    let mut current: Option<Group> = None;
    let mut rules_started = false;
    for (index, raw) in body.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            if let Some(group) = current.take() {
                groups.push(group);
            }
            rules_started = false;
            continue;
        }
        let Some((name, value)) = split_field(line) else {
            continue;
        };
        if name.eq_ignore_ascii_case("user-agent") {
            if rules_started {
                if let Some(group) = current.take() {
                    groups.push(group);
                }
                rules_started = false;
            }
            let group = current.get_or_insert_with(|| Group {
                agents: Vec::new(),
                rules: Vec::new(),
                crawl_delay: None,
            });
            group.agents.push(value.to_string());
            continue;
        }
        let Some(group) = current.as_mut() else {
            continue;
        };
        rules_started = true;
        if name.eq_ignore_ascii_case("allow") {
            group.rules.push(Rule {
                allow: true,
                pattern: value.to_string(),
                line: index,
            });
        } else if name.eq_ignore_ascii_case("disallow") {
            if value.is_empty() {
                group.rules.push(Rule {
                    allow: true,
                    pattern: String::new(),
                    line: index,
                });
            } else {
                group.rules.push(Rule {
                    allow: false,
                    pattern: value.to_string(),
                    line: index,
                });
            }
        } else if name.eq_ignore_ascii_case("crawl-delay") {
            if let Some(seconds) = value
                .parse::<f64>()
                .ok()
                .filter(|item| item.is_finite() && *item >= 0.0)
            {
                let seconds = seconds.min(86_400.0);
                group.crawl_delay = Some(Duration::from_secs_f64(seconds));
            }
        }
    }
    if let Some(group) = current {
        groups.push(group);
    }
    groups
}

pub fn decide(groups: &[Group], user_agent: &str, path: &str) -> (Decision, Option<Duration>) {
    let Some(group) = select(groups, product_token(user_agent)) else {
        return (Decision::Allow, None);
    };
    let mut best: Option<&Rule> = None;
    for rule in &group.rules {
        if !path_matches(&rule.pattern, path) {
            continue;
        }
        let replace = match best {
            None => true,
            Some(current) => {
                let longer = rule.pattern.len() > current.pattern.len();
                let earlier =
                    rule.pattern.len() == current.pattern.len() && rule.line < current.line;
                longer || earlier
            }
        };
        if replace {
            best = Some(rule);
        }
    }
    let decision = match best {
        Some(rule) if !rule.allow => Decision::Disallow,
        _ => Decision::Allow,
    };
    (decision, group.crawl_delay)
}

pub fn product_token(user_agent: &str) -> Option<&str> {
    let token = user_agent.split_whitespace().next()?;
    let name = token.split('/').next().unwrap_or("");
    if name.is_empty() { None } else { Some(name) }
}

fn select<'a>(groups: &'a [Group], token: Option<&str>) -> Option<&'a Group> {
    let mut specific: Option<&Group> = None;
    let mut specific_len = 0usize;
    let mut wildcard: Option<&Group> = None;
    for group in groups {
        for agent in &group.agents {
            if agent == "*" {
                if wildcard.is_none() {
                    wildcard = Some(group);
                }
                continue;
            }
            if token.is_some_and(|token| token.eq_ignore_ascii_case(agent))
                && agent.len() > specific_len
            {
                specific = Some(group);
                specific_len = agent.len();
            }
        }
    }
    specific.or(wildcard)
}

fn path_matches(pattern: &str, path: &str) -> bool {
    let end = pattern.ends_with('$');
    let pattern = pattern.trim_end_matches('$');
    if !pattern.contains('*') {
        return if end {
            path == pattern
        } else {
            path.starts_with(pattern)
        };
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if end {
        anchored(path, &parts)
    } else {
        parts_prefix(path, &parts)
    }
}

fn parts_prefix(path: &str, parts: &[&str]) -> bool {
    let mut rest = path;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if index == 0 {
            if !rest.starts_with(part) {
                return false;
            }
            rest = &rest[part.len()..];
        } else if let Some(found) = rest.find(part) {
            rest = &rest[found + part.len()..];
        } else {
            return false;
        }
    }
    true
}

fn anchored(path: &str, parts: &[&str]) -> bool {
    if !parts_prefix(path, parts) {
        return false;
    }
    let mut rest = path;
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if index == 0 {
            rest = &rest[part.len()..];
        } else if let Some(found) = rest.find(part) {
            rest = &rest[found + part.len()..];
        }
    }
    let last = parts.last().copied().unwrap_or("");
    last.is_empty() || path.ends_with(last) && rest.is_empty()
}

fn split_field(line: &str) -> Option<(&str, &str)> {
    let (name, value) = line.split_once(':')?;
    Some((name.trim(), value.trim()))
}
