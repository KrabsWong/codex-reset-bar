//! The shapes of `https://codex-resets.com/api/openapi.json`.
//!
//! Fields are optional wherever the API allows null, so a later revision
//! that adds or relaxes a field does not break the panel.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResetType {
    Regular,
    Banked,
}

impl ResetType {
    pub fn label(self) -> &'static str {
        match self {
            ResetType::Regular => "regular",
            ResetType::Banked => "banked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ResetSource {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Reset {
    pub id: String,
    pub reset_type: ResetType,
    pub announced_at: String,
    pub text: String,
    #[serde(default)]
    pub source: Option<ResetSource>,
}

impl Reset {
    pub fn url(&self) -> Option<&str> {
        self.source.as_ref()?.url.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ScheduledReset {
    pub id: String,
    pub reset_type: ResetType,
    pub announced_at: String,
    #[serde(default)]
    pub scheduled_for: Option<String>,
    pub text: String,
    #[serde(default)]
    pub source: Option<ResetSource>,
}

impl ScheduledReset {
    pub fn url(&self) -> Option<&str> {
        self.source.as_ref()?.url.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Watch {
    pub level: String,
    #[serde(default)]
    pub reset_chance_percent: Option<u32>,
    #[serde(default)]
    pub forecast_window: Option<String>,
    pub expires_at: String,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Stats {
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub last_reset_at: Option<String>,
    #[serde(default)]
    pub days_since_last: Option<f64>,
    #[serde(default)]
    pub avg_interval_days: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct StatusData {
    #[serde(default)]
    pub latest_reset: Option<Reset>,
    #[serde(default)]
    pub scheduled_reset: Option<ScheduledReset>,
    #[serde(default)]
    pub active_watch: Option<Watch>,
    pub stats: Stats,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Meta {
    #[serde(default)]
    pub api_version: String,
    #[serde(default)]
    pub generated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct StatusEnvelope {
    pub data: StatusData,
    #[serde(default)]
    pub meta: Option<Meta>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Pagination {
    pub has_more: bool,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct ResetPage {
    pub data: Vec<Reset>,
    pub pagination: Pagination,
    #[serde(default)]
    pub meta: Option<Meta>,
}

/// Maximum completed interval, only when all history is available.
pub fn longest_wait_days(resets: &[Reset], has_more: bool) -> Option<f64> {
    if has_more {
        return None;
    }
    let mut times = resets.iter()
        .map(|reset| crate::format::parse(&reset.announced_at))
        .collect::<Option<Vec<_>>>()?;
    times.sort_unstable();
    times.windows(2)
        .map(|pair| pair[1] - pair[0])
        .max()
        .map(|seconds| seconds as f64 / 86_400.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset(at: &str) -> Reset {
        Reset {
            id: at.to_owned(), reset_type: ResetType::Regular,
            announced_at: at.to_owned(), text: String::new(), source: None,
        }
    }

    #[test]
    fn longest_wait_uses_sorted_intervals_including_banked() {
        let mut banked = reset("2026-01-03T12:00:00Z");
        banked.reset_type = ResetType::Banked;
        let rows = [reset("2026-01-04T12:00:00Z"), reset("2026-01-01T00:00:00Z"),
            banked, reset("2026-01-01T00:00:00Z")];
        assert_eq!(longest_wait_days(&rows, false), Some(2.5));
        assert_eq!(longest_wait_days(&rows, true), None);
    }

    #[test]
    fn longest_wait_does_not_invent_values_for_missing_or_invalid_history() {
        assert_eq!(longest_wait_days(&[], false), None);
        assert_eq!(longest_wait_days(&[reset("2026-01-01T00:00:00Z")], false), None);
        assert_eq!(longest_wait_days(&[reset("invalid"), reset("2026-01-01T00:00:00Z")], false), None);
    }
}
