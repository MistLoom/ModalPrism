//! 钩子通道分拣（Mod / DSH 共用规则，与具体 Ability 解耦）。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookKind {
    Invoke,
    Bus,
    User,
}

pub fn classify_hook(name: &str) -> Option<HookKind> {
    if name.starts_with("ui.")
        || name.starts_with("prompt.")
        || name == "session.start"
        || name == "session.end"
    {
        return Some(HookKind::User);
    }
    if name.contains('/') && !name.starts_with("tool.") {
        return Some(HookKind::Bus);
    }
    if name.starts_with("tool.") {
        return Some(HookKind::Invoke);
    }
    match name {
        "tool.call" | "tool.check" | "tool.result" => Some(HookKind::Invoke),
        "turn.complete" | "turn.start" => Some(HookKind::Bus),
        "SessionStart" | "SessionEnd" | "Stop" | "PreToolUse" | "PostToolUse"
        | "UserPromptSubmit" => Some(HookKind::User),
        _ => Some(HookKind::Invoke),
    }
}
