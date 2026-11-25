use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct User {
    pub id: i64,
    pub display_name: String,
    pub username: String,
    pub group: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Balance {
    pub quota: i64,
    pub used_quota: i64,
    pub request_count: u64,
    pub group: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Announcement {
    pub content: String,
    #[serde(rename = "publishDate")]
    pub publish_date: String,
}

impl Announcement {
    pub fn timestamp(&self) -> String {
        DateTime::parse_from_rfc3339(&self.publish_date)
            .map(|date| {
                date.with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|_| self.publish_date.clone())
    }
}

#[derive(Deserialize)]
pub struct Status {
    #[serde(default)]
    pub announcements: Vec<Announcement>,
}

/// Format quota in dollars, rounding to cents with exact integer arithmetic.
/// One cent is 5,000 quota units. Ties round away from zero.
pub fn quota_units(quota: i64) -> String {
    let absolute = i128::from(quota).abs();
    let cents = (absolute + 2_500) / 5_000;
    let sign = if quota < 0 && cents != 0 { "-" } else { "" };
    format!("${sign}{}.{:02}", cents / 100, cents % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_rounding_matches_documented_values_and_boundaries() {
        for (input, expected) in [
            (6_532_248_664, "$13064.50"),
            (45_117_751_336, "$90235.50"),
            (0, "$0.00"),
            (2_499, "$0.00"),
            (2_500, "$0.01"),
            (497_499, "$0.99"),
            (497_500, "$1.00"),
            (500_000, "$1.00"),
            (-2_499, "$0.00"),
            (-2_500, "$-0.01"),
            (-497_500, "$-1.00"),
            (i64::MAX, "$18446744073709.55"),
            (i64::MIN, "$-18446744073709.55"),
        ] {
            assert_eq!(quota_units(input), expected);
        }
    }

    #[test]
    fn timestamp_keeps_seconds_and_handles_malformed_dates() {
        let announcement = Announcement {
            content: String::new(),
            publish_date: "2026-11-14T18:54:11.543Z".into(),
        };
        let formatted = announcement.timestamp();
        assert_eq!(formatted.len(), 19);
        assert!(formatted.ends_with(":11"));
        assert_eq!(
            Announcement {
                publish_date: "unknown".into(),
                ..announcement
            }
            .timestamp(),
            "unknown"
        );
    }
}
