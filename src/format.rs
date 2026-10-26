//! Time and number formatting for the panel: everything is stored as an
//! epoch second, shown in the user's own time zone.

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
        .map(|time| time.format("%b %-d, %Y · %H:%M").to_string())
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
    let delta = now - timestamp;
    let seconds = delta.unsigned_abs();
    if seconds < 60 {
        return if delta < 0 { "Soon" } else { "Just now" }.to_owned();
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
    let seconds = target - now;
    if seconds <= 0 {
        return "Awaiting reset".to_owned();
    }
    let days = seconds / 86400;
    let hours = seconds % 86400 / 3600;
    let minutes = seconds % 3600 / 60;
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
    fn english_relative_times_and_plurals() {
        assert_eq!(relative(0, 3600), "1 hr ago");
        assert_eq!(relative(0, 172800), "2 days ago");
        assert_eq!(relative(86400, 0), "In 1 day");
        assert_eq!(relative(0, 0), "Just now");
    }
    #[test]
    fn countdown_does_not_claim_execution() {
        assert_eq!(countdown(0, 1), "Awaiting reset");
        assert_eq!(countdown(3660, 0), "In 1h 1m");
        assert_eq!(countdown(1, 0), "In 1 min");
    }
}
