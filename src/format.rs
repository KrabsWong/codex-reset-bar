//! Time and number formatting for the panel: everything is stored as an
//! epoch second, shown in the user's own time zone.

use crate::i18n::{self, tr};
use crate::settings::Language;
use chrono::{DateTime, Local, TimeZone};

pub fn now_epoch() -> i64 {
    Local::now().timestamp()
}

pub fn parse(timestamp: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|parsed| parsed.timestamp())
}

fn local(timestamp: i64) -> Option<DateTime<Local>> {
    Local.timestamp_opt(timestamp, 0).single()
}

/// `Oct 25, 2026 · 14:06`
pub fn absolute(timestamp: i64) -> String {
    local(timestamp)
        .map(|time| time.format(tr("%b %-d, %Y · %H:%M", "%Y年%-m月%-d日 · %H:%M")).to_string())
        .unwrap_or_else(|| "—".to_owned())
}

/// `10-25` and `14:06` for the dense history rows.
pub fn short_date(timestamp: i64) -> String {
    local(timestamp)
        .map(|time| time.format("%m-%d").to_string())
        .unwrap_or_else(|| "—".to_owned())
}

/// `14:06`
pub fn clock_time(timestamp: i64) -> String {
    local(timestamp)
        .map(|time| time.format("%H:%M").to_string())
        .unwrap_or_else(|| "—".to_owned())
}

/// Compact relative time for menu rows, rounded down to whole units.
pub fn relative(timestamp: i64, now: i64) -> String {
    relative_in(timestamp, now, i18n::language())
}

fn relative_in(timestamp: i64, now: i64, language: Language) -> String {
    let delta = now - timestamp;
    let seconds = delta.unsigned_abs();
    if seconds < 60 {
        return match (language, delta < 0) {
            (Language::English, true) => "Soon", (Language::English, false) => "Just now",
            (Language::SimplifiedChinese, true) => "即将", (Language::SimplifiedChinese, false) => "刚刚",
        }.to_owned();
    }
    let (value, unit) = if seconds < 3600 {
        (seconds / 60, "min")
    } else if seconds < 86400 {
        (seconds / 3600, "hr")
    } else if seconds < 30 * 86400 {
        (seconds / 86400, "day")
    } else if seconds < 365 * 86400 {
        (seconds / (30 * 86400), "month")
    } else {
        (seconds / (365 * 86400), "year")
    };
    if language == Language::SimplifiedChinese {
        let unit = match unit { "min" => "分钟", "hr" => "小时", "day" => "天", "month" => "个月", _ => "年" };
        return format!("{value}{unit}{}", if delta < 0 { "后" } else { "前" });
    }
    let plural = if value > 1 && !matches!(unit, "min" | "hr") {
        "s"
    } else {
        ""
    };
    if delta < 0 {
        format!("In {value} {unit}{plural}")
    } else {
        format!("{value} {unit}{plural} ago")
    }
}

/// A compact countdown, without implying a scheduled reset has occurred.
pub fn countdown(target: i64, now: i64) -> String {
    countdown_in(target, now, i18n::language())
}

fn countdown_in(target: i64, now: i64, language: Language) -> String {
    let seconds = target - now;
    if seconds <= 0 {
        return if language == Language::English { "Awaiting reset" } else { "等待重置" }.to_owned();
    }
    let days = seconds / 86400;
    let hours = seconds % 86400 / 3600;
    let minutes = seconds % 3600 / 60;
    if language == Language::SimplifiedChinese {
        return match (days, hours) {
            (0, 0) => format!("{} 分钟后", minutes.max(1)),
            (0, h) => format!("{h} 小时 {minutes} 分钟后"),
            (d, h) => format!("{d} 天 {h} 小时后"),
        };
    }
    match (days, hours) {
        (0, 0) => format!("In {} min", minutes.max(1)),
        (0, h) => format!("In {h}h {minutes}m"),
        (d, h) => format!("In {d}d {h}h"),
    }
}

/// `6.1`
pub fn number(value: f64) -> String {
    if (value - value.round()).abs() < 0.05 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chinese_relative_times_and_countdown() {
        assert_eq!(relative_in(0, 3600, Language::SimplifiedChinese), "1小时前");
        assert_eq!(relative_in(86400, 0, Language::SimplifiedChinese), "1天后");
        assert_eq!(relative_in(0, 0, Language::SimplifiedChinese), "刚刚");
        assert_eq!(countdown_in(0, 1, Language::SimplifiedChinese), "等待重置");
        assert_eq!(countdown_in(3660, 0, Language::SimplifiedChinese), "1 小时 1 分钟后");
    }
    #[test]
    fn english_relative_times_and_plurals() {
        assert_eq!(relative_in(0, 3600, Language::English), "1 hr ago");
        assert_eq!(relative_in(0, 172800, Language::English), "2 days ago");
        assert_eq!(relative_in(86400, 0, Language::English), "In 1 day");
        assert_eq!(relative_in(0, 0, Language::English), "Just now");
    }
    #[test]
    fn countdown_does_not_claim_execution() {
        assert_eq!(countdown_in(0, 1, Language::English), "Awaiting reset");
        assert_eq!(countdown_in(3660, 0, Language::English), "In 1h 1m");
        assert_eq!(countdown_in(1, 0, Language::English), "In 1 min");
    }
}
