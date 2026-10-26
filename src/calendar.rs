//! Local-date buckets for the contribution calendar. Pagination gaps are
//! unknown, never presented as days without a reset.
use crate::model::{Reset, ResetType};
use chrono::{Datelike, Duration, Local, NaiveDate};

pub const WEEKS: usize = 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub regular: usize,
    pub banked: usize,
    pub known: bool,
    pub future: bool,
    pub url: Option<String>,
}

impl Day {
    pub fn hint(&self) -> String {
        let detail = if self.future {
            "Future date".to_owned()
        } else if self.regular + self.banked > 0 {
            format!("{} regular, {} banked", self.regular, self.banked)
        } else if self.known {
            "No resets".to_owned()
        } else {
            "History not loaded".to_owned()
        };
        format!("{} · {}", self.date, detail)
    }
}

pub fn days(rows: &[Reset], has_more: bool, today: NaiveDate) -> Vec<Day> {
    let events: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            chrono::DateTime::parse_from_rfc3339(&row.announced_at)
                .ok()
                .map(|at| (at.with_timezone(&Local).date_naive(), row))
        })
        .collect();
    let oldest = events.iter().map(|(date, _)| *date).min();
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let start = monday - Duration::weeks((WEEKS - 1) as i64);
    (0..WEEKS * 7)
        .map(|index| {
            let date = start + Duration::days(index as i64);
            let mut day = Day {
                date,
                regular: 0,
                banked: 0,
                known: !has_more || oldest.is_some_and(|oldest| date > oldest),
                future: date > today,
                url: None,
            };
            for (_, row) in events.iter().filter(|(at, _)| *at == date) {
                match row.reset_type {
                    ResetType::Regular => day.regular += 1,
                    ResetType::Banked => day.banked += 1,
                }
                if day.url.is_none() {
                    day.url = row.url().map(str::to_owned);
                }
            }
            day
        })
        .collect()
}

pub fn today(epoch: i64) -> NaiveDate {
    chrono::DateTime::from_timestamp(epoch, 0)
        .expect("valid application timestamp")
        .with_timezone(&Local)
        .date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    fn row(date: NaiveDate, kind: ResetType) -> Reset {
        Reset {
            id: format!("{date}{kind:?}"),
            reset_type: kind,
            announced_at: Local
                .from_local_datetime(&date.and_hms_opt(12, 0, 0).unwrap())
                .single()
                .unwrap()
                .to_rfc3339(),
            text: String::new(),
            source: None,
        }
    }
    #[test]
    fn weeks_align_and_future_is_not_empty_history() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let grid = days(&[], false, today);
        assert_eq!(grid.len(), 140);
        assert_eq!(grid[0].date.weekday(), chrono::Weekday::Mon);
        assert_eq!(grid[133].date, today);
        assert!(!grid[133].future);
        assert!(grid[134].future);
    }
    #[test]
    fn pagination_does_not_invent_empty_days() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let rows = [row(today - Duration::days(2), ResetType::Regular)];
        let partial = days(&rows, true, today);
        assert!(!partial[130].known);
        assert!(partial[132].known);
        assert!(days(&rows, false, today)[130].known);
    }
    #[test]
    fn counts_both_reset_types_on_the_same_local_day() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let rows = [
            row(today, ResetType::Regular),
            row(today, ResetType::Banked),
        ];
        let grid = days(&rows, false, today);
        assert_eq!((grid[133].regular, grid[133].banked), (1, 1));
    }
    #[test]
    fn empty_day_hint_requires_loaded_history() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        assert_eq!(days(&[], false, today)[0].hint(), "2026-06-15 · No resets");
        assert_eq!(
            days(&[], true, today)[0].hint(),
            "2026-06-15 · History not loaded"
        );
        let rows = [row(today - Duration::weeks(21), ResetType::Regular)];
        assert!(days(&rows, true, today)[0].known);
    }
}
