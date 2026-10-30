//! 声明里的时长。
//!
//! `30s`、`2h`、`365d` 这类写法在配置、调度与内容投递里通用，只在这里解析一次。

use std::time::Duration;

/// 解析 `<正整数><ms|s|m|h|d>`。单位之外的内容一律不接受。
pub fn parse(text: &str) -> Option<Duration> {
    let split = text.find(|ch: char| !ch.is_ascii_digit())?;
    let count: u64 = text[..split].parse().ok()?;
    let scale = match &text[split..] {
        "ms" => 1,
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    Some(Duration::from_millis(count.saturating_mul(scale)))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn units_are_exact() {
        assert_eq!(parse("250ms").unwrap().as_millis(), 250);
        assert_eq!(parse("2h").unwrap().as_secs(), 7_200);
        assert!(parse("2").is_none());
        assert!(parse("2w").is_none());
    }
}
