// SPDX-License-Identifier: MIT
//! Time bands for the chooser: the Day | Week | Month | All chips and the headings rows are
//! grouped under (Today, Yesterday, Earlier this week, Earlier this month, Older). Plain
//! functions, so the window and the tests use the same maths.

use chrono::{DateTime, Local, NaiveTime, TimeDelta, TimeZone};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chip {
    Day,
    Week,
    Month,
    All,
}

impl Chip {
    pub const EVERY: [Chip; 4] = [Chip::Day, Chip::Week, Chip::Month, Chip::All];

    pub fn label(self) -> &'static str {
        match self {
            Chip::Day => "Day",
            Chip::Week => "Week",
            Chip::Month => "Month",
            Chip::All => "All",
        }
    }

    /// The word kept in ui.json.
    pub fn key(self) -> &'static str {
        match self {
            Chip::Day => "day",
            Chip::Week => "week",
            Chip::Month => "month",
            Chip::All => "all",
        }
    }

    pub fn from_key(k: &str) -> Option<Chip> {
        Chip::EVERY.into_iter().find(|c| c.key() == k)
    }

    /// True when `t` falls inside the chip: the last 24 hours, 7 days or 30 days (or always).
    pub fn holds<Tz: TimeZone>(self, now: &DateTime<Tz>, t: &DateTime<Tz>) -> bool {
        let age = now.clone().signed_duration_since(t.clone());
        match self {
            Chip::Day => age < TimeDelta::hours(24),
            Chip::Week => age < TimeDelta::days(7),
            Chip::Month => age < TimeDelta::days(30),
            Chip::All => true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Band {
    Today,
    Yesterday,
    Week,
    Month,
    Older,
}

impl Band {
    pub fn label(self) -> &'static str {
        match self {
            Band::Today => "Today",
            Band::Yesterday => "Yesterday",
            Band::Week => "Earlier this week",
            Band::Month => "Earlier this month",
            Band::Older => "Older",
        }
    }
}

/// Start of a local day (the earliest instant, so a DST change cannot skip it).
fn day_start<Tz: TimeZone>(tz: &Tz, day: chrono::NaiveDate) -> Option<DateTime<Tz>> {
    tz.from_local_datetime(&day.and_time(NaiveTime::MIN)).earliest()
}

/// The heading a time sits under: Today and Yesterday follow local midnight, the rest the age
/// (7 and 30 days). A time in the future counts as Today.
pub fn band<Tz: TimeZone>(now: &DateTime<Tz>, t: &DateTime<Tz>) -> Band {
    let tz = now.timezone();
    let today = now.date_naive();
    if day_start(&tz, today).is_none_or(|m| *t >= m) {
        return Band::Today;
    }
    if today.pred_opt().and_then(|d| day_start(&tz, d)).is_none_or(|m| *t >= m) {
        return Band::Yesterday;
    }
    let age = now.clone().signed_duration_since(t.clone());
    if age < TimeDelta::days(7) {
        Band::Week
    } else if age < TimeDelta::days(30) {
        Band::Month
    } else {
        Band::Older
    }
}

/// Seconds since the epoch of an ISO time (`2026-11-16T10:00:00Z`), or None.
pub fn parse_iso(t: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(t).ok().map(|d| d.timestamp())
}

/// Seconds since the epoch as local time.
pub fn local(secs: i64) -> DateTime<Local> {
    Local.timestamp_opt(secs, 0).earliest().unwrap_or_else(Local::now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, Utc};

    fn at(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    #[test]
    fn bands_follow_local_midnight_then_age() {
        let now = at("2026-11-19T10:00:00+01:00");
        assert_eq!(band(&now, &at("2026-11-19T00:00:00+01:00")), Band::Today);
        assert_eq!(band(&now, &at("2026-11-20T08:00:00+01:00")), Band::Today); // future
        assert_eq!(band(&now, &at("2026-11-18T23:59:59+01:00")), Band::Yesterday);
        assert_eq!(band(&now, &at("2026-11-18T00:00:00+01:00")), Band::Yesterday);
        assert_eq!(band(&now, &at("2026-11-17T23:59:59+01:00")), Band::Week);
        assert_eq!(band(&now, &at("2026-11-12T10:00:01+01:00")), Band::Week);
        assert_eq!(band(&now, &at("2026-11-12T10:00:00+01:00")), Band::Month); // exactly 7 days
        assert_eq!(band(&now, &at("2026-10-20T10:00:01+01:00")), Band::Month);
        assert_eq!(band(&now, &at("2026-10-20T10:00:00+01:00")), Band::Older); // exactly 30 days
    }

    #[test]
    fn midnight_is_local_not_utc() {
        // 23:30 UTC on the 18th is already the 19th at +01:00
        let now = at("2026-11-19T10:00:00+01:00");
        let t = at("2026-11-18T23:30:00Z").with_timezone(&FixedOffset::east_opt(3600).unwrap());
        assert_eq!(band(&now, &t), Band::Today);
        let now_utc = now.with_timezone(&Utc);
        assert_eq!(band(&now_utc, &t.with_timezone(&Utc)), Band::Yesterday);
    }

    #[test]
    fn chips_are_24h_7d_30d() {
        let now = at("2026-11-19T10:00:00+01:00");
        let t = |s| at(s);
        assert!(Chip::Day.holds(&now, &t("2026-11-18T10:00:01+01:00")));
        assert!(!Chip::Day.holds(&now, &t("2026-11-18T10:00:00+01:00")));
        assert!(Chip::Week.holds(&now, &t("2026-11-12T10:00:01+01:00")));
        assert!(!Chip::Week.holds(&now, &t("2026-11-12T10:00:00+01:00")));
        assert!(Chip::Month.holds(&now, &t("2026-10-20T10:00:01+01:00")));
        assert!(!Chip::Month.holds(&now, &t("2026-10-20T10:00:00+01:00")));
        assert!(Chip::All.holds(&now, &t("2000-01-01T00:00:00Z")));
        assert_eq!(Chip::from_key("week"), Some(Chip::Week));
        assert_eq!(Chip::from_key("x"), None);
        assert_eq!(parse_iso("2026-11-16T10:00:00Z"), Some(1_794_823_200));
        assert_eq!(parse_iso("nope"), None);
    }
}
