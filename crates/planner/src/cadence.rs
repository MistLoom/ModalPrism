//! 自适应间隔。单位与设计公式一致：1440 是一天的分钟数。

use std::time::Duration;

#[derive(Clone, Debug)]
pub struct CadenceInput {
    pub yield_7d: u64,
    pub floor: Duration,
    pub ceiling: Duration,
    pub signal: Duration,
    pub paid: bool,
    pub adaptive: bool,
    pub fixed: Duration,
}

pub fn next_interval(input: &CadenceInput) -> Duration {
    let floor = if input.paid {
        input.floor.max(Duration::from_secs(3600))
    } else {
        input.floor
    };
    if !input.adaptive {
        return input.fixed.clamp(floor, input.ceiling.max(floor));
    }
    let adaptive = if input.yield_7d == 0 {
        input.ceiling
    } else {
        let per_day = input.yield_7d as f64 / 7.0;
        let minutes = 1440.0 / (per_day * 3.0);
        Duration::from_secs_f64((minutes * 60.0).max(0.0))
    };
    let raw = adaptive.max(input.signal).max(floor);
    raw.clamp(floor, input.ceiling.max(floor))
}

pub fn signal_of(parts: &[Option<Duration>]) -> Duration {
    parts
        .iter()
        .flatten()
        .copied()
        .max()
        .unwrap_or(Duration::ZERO)
}
