//! Partial release dates and provider timestamps.
//!
//! Every provider stamps when something was added or changed, and each spells it a little
//! differently, so one tolerant reader serves them all rather than one per provider.

use serde::{Deserialize, Serialize};

/// A release date with the precision supplied by the provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReleaseDate {
    Year(u32),
    YearMonth { year: u32, month: u8 },
    YearMonthDay { year: u32, month: u8, day: u8 },
}

impl ReleaseDate {
    /// Builds a date from the longest valid prefix of its components.
    pub fn from_parts(year: i32, month: i32, day: i32) -> Option<Self> {
        if !(1..=9999).contains(&year) {
            return None;
        }
        let year = year as u32;
        if !(1..=12).contains(&month) {
            return Some(Self::Year(year));
        }
        let month = month as u8;
        if jiff::civil::Date::new(year as i16, month as i8, day.try_into().ok().unwrap_or(0))
            .is_ok()
        {
            Some(Self::YearMonthDay {
                year,
                month,
                day: day as u8,
            })
        } else {
            Some(Self::YearMonth { year, month })
        }
    }

    /// Reads a partial or full ISO date, ignoring a time suffix.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().split(['T', ' ']).next()?;
        let mut parts = value.split('-');
        let year = parts.next()?;
        if year.len() != 4 {
            return None;
        }
        let month = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
        let day = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
        Self::from_parts(year.parse().ok()?, month, day)
    }

    /// Returns the date components in chronological sort order.
    pub fn parts(self) -> (u32, Option<u8>, Option<u8>) {
        match self {
            Self::Year(year) => (year, None, None),
            Self::YearMonth { year, month } => (year, Some(month), None),
            Self::YearMonthDay { year, month, day } => (year, Some(month), Some(day)),
        }
    }

    pub fn year(self) -> u32 {
        self.parts().0
    }

    /// Checks whether a more precise date agrees with all the known components.
    pub(crate) fn is_prefix_of(self, other: Self) -> bool {
        let (year, month, day) = self.parts();
        let (other_year, other_month, other_day) = other.parts();
        year == other_year
            && month.is_none_or(|month| Some(month) == other_month)
            && day.is_none_or(|day| Some(day) == other_day)
    }
}

impl Ord for ReleaseDate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.parts().cmp(&other.parts())
    }
}

impl PartialOrd for ReleaseDate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// An ISO-8601 stamp as seconds since the epoch. It takes a bare date (`2026-09-15`), a date and
/// time joined by `T` or a space, fractional seconds, and a `Z`, `+02:00` or `+0000` offset,
/// which covers what Apple Music, Deezer and Tidal write. A missing month or day reads as the
/// first. `None` when not even the year is a number.
pub fn seconds(stamp: &str) -> Option<i64> {
    let stamp = stamp.trim();
    let (date, time) = stamp.split_once(['T', ' ']).unwrap_or((stamp, ""));
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next().unwrap_or("1").parse().unwrap_or(1);
    let day: i64 = parts.next().unwrap_or("1").parse().unwrap_or(1);
    let (time, zone) = zone_of(time.trim_end_matches('Z'));
    let mut clock = time.split(':');
    let hour: i64 = clock.next().unwrap_or("0").parse().unwrap_or(0);
    let minute: i64 = clock.next().unwrap_or("0").parse().unwrap_or(0);
    let second: i64 = clock
        .next()
        .and_then(|second| second.split('.').next())
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    Some(days(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second - zone)
}

/// Days from the epoch to a civil date, by Howard Hinnant's algorithm.
fn days(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let shifted = match month > 2 {
        true => month - 3,
        false => month + 9,
    };
    let doy = (153 * shifted + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Splits a time of day from the offset after it, as seconds east of UTC. A time with no offset
/// is UTC.
fn zone_of(time: &str) -> (&str, i64) {
    let Some(at) = time.rfind(['+', '-']).filter(|&at| at > 0) else {
        return (time, 0);
    };
    let (clock, zone) = time.split_at(at);
    let sign = match zone.starts_with('-') {
        true => -1i64,
        false => 1,
    };
    let zone = zone.trim_start_matches(['+', '-']);
    // `+02:00` and `+0200` both occur, so the hours are the first two digits either way.
    let (hours, minutes) = match zone.split_once(':') {
        Some((hours, minutes)) => (hours, minutes),
        None => zone
            .split_at_checked(zone.len().min(2))
            .unwrap_or((zone, "")),
    };
    let hours: i64 = hours.parse().unwrap_or(0);
    let minutes: i64 = minutes.parse().unwrap_or(0);
    (clock, sign * (hours * 3_600 + minutes * 60))
}
