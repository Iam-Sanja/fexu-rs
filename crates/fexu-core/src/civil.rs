//! Local civil time for an optional project zone.
//!
//! `NaiveDate` stays a calendar day. `DateTime<Utc>` stays an absolute instant.
//! A `datetime-local` value is civil time. Without `FEXU_TIMEZONE` that civil
//! time is UTC, which is the historical form contract. A configured IANA zone
//! converts it and never treats the same digits as UTC.

use chrono::{DateTime, LocalResult, NaiveDateTime, TimeZone, Timelike, Utc};

use crate::temporal::{
    datetime_local_value, format_datetime, parse_datetime_local, parse_datetime_utc,
};

/// How an ambiguous civil time is resolved. Absent means the caller has not chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CivilFold {
    Earlier,
    Later,
}

impl CivilFold {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value.trim() {
            "earlier" => Ok(Self::Earlier),
            "later" => Ok(Self::Later),
            _ => Err("validation.datetime_fold"),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Earlier => "earlier",
            Self::Later => "later",
        }
    }
}

/// Value written into `datetime-local`, plus the fold when that clock time occurs twice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalDateTimeInput {
    pub value: String,
    pub fold: Option<CivilFold>,
}

/// Project zone for form and admin civil times. The default is UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectTimeZone {
    kind: ZoneKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZoneKind {
    Utc,
    Iana(chrono_tz::Tz),
}

impl ProjectTimeZone {
    pub const fn utc() -> Self {
        Self {
            kind: ZoneKind::Utc,
        }
    }

    pub const fn is_utc(self) -> bool {
        matches!(self.kind, ZoneKind::Utc)
    }

    /// IANA name, or `UTC` when no zone is configured.
    pub fn label(self) -> &'static str {
        match self.kind {
            ZoneKind::Utc => "UTC",
            ZoneKind::Iana(zone) => zone.name(),
        }
    }

    /// `UTC`, `Etc/UTC` and an empty string keep the UTC contract.
    /// Any other value must be the exact IANA name, for example `Europe/Berlin`.
    pub fn parse(name: &str) -> Result<Self, &'static str> {
        let name = name.trim();
        if name.is_empty() || name.eq_ignore_ascii_case("utc") || name == "Etc/UTC" {
            return Ok(Self::utc());
        }
        let zone: chrono_tz::Tz = name.parse().map_err(|_| "validation.timezone")?;
        if zone == chrono_tz::UTC {
            return Ok(Self::utc());
        }
        Ok(Self {
            kind: ZoneKind::Iana(zone),
        })
    }
}

/// `FEXU_TIMEZONE` unset or empty means UTC. An unknown name is an error.
pub fn project_zone_from_env() -> Result<ProjectTimeZone, &'static str> {
    match std::env::var("FEXU_TIMEZONE") {
        Err(_) => Ok(ProjectTimeZone::utc()),
        Ok(value) if value.trim().is_empty() => Ok(ProjectTimeZone::utc()),
        Ok(value) => ProjectTimeZone::parse(&value),
    }
}

/// Converts one civil `datetime-local` value to a UTC instant.
///
/// A `Z` or numeric offset is rejected. The zone, not the offset, owns the conversion.
/// A spring gap is `validation.datetime_gap`. An autumn overlap without `fold` is
/// `validation.datetime_ambiguous`. `fold` on a unique time is `validation.datetime_fold`.
pub fn parse_civil_datetime(
    value: &str,
    zone: ProjectTimeZone,
    fold: Option<CivilFold>,
) -> Result<DateTime<Utc>, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        return Err("validation.required");
    }
    if value.contains('Z') || value.contains('+') || value.matches('-').count() > 2 {
        return Err("validation.datetime_offset");
    }
    let naive = parse_naive(value)?;
    match zone.kind {
        ZoneKind::Utc => {
            if fold.is_some() {
                return Err("validation.datetime_fold");
            }
            Ok(naive.and_utc())
        }
        ZoneKind::Iana(zone) => match zone.from_local_datetime(&naive) {
            LocalResult::None => Err("validation.datetime_gap"),
            LocalResult::Single(local) => {
                if fold.is_some() {
                    Err("validation.datetime_fold")
                } else {
                    Ok(local.with_timezone(&Utc))
                }
            }
            LocalResult::Ambiguous(early, late) => match fold {
                None => Err("validation.datetime_ambiguous"),
                Some(CivilFold::Earlier) => Ok(early.with_timezone(&Utc)),
                Some(CivilFold::Later) => Ok(late.with_timezone(&Utc)),
            },
        },
    }
}

/// Admin and filter entry. UTC keeps the historical parsers.
/// A configured zone reads civil time and still accepts an explicit UTC instant.
pub fn parse_form_datetime(
    value: &str,
    zone: ProjectTimeZone,
    fold: Option<CivilFold>,
) -> Result<DateTime<Utc>, &'static str> {
    match zone.kind {
        ZoneKind::Utc => {
            if fold.is_some() {
                return Err("validation.datetime_fold");
            }
            parse_datetime_local(value).or_else(|_| parse_datetime_utc(value))
        }
        ZoneKind::Iana(_) => parse_civil_datetime(value, zone, fold),
    }
}

/// Filter values. UTC still requires an absolute instant.
/// A configured zone accepts civil time, and an explicit `Z` or `+00:00` instant.
pub fn parse_filter_datetime(
    value: &str,
    zone: ProjectTimeZone,
) -> Result<DateTime<Utc>, &'static str> {
    match zone.kind {
        ZoneKind::Utc => parse_datetime_utc(value),
        ZoneKind::Iana(_) => match parse_civil_datetime(value, zone, None) {
            Err("validation.datetime_offset") => parse_datetime_utc(value),
            other => other,
        },
    }
}

/// Renders the civil clock that parses back to `instant`.
pub fn format_civil_datetime(instant: DateTime<Utc>, zone: ProjectTimeZone) -> LocalDateTimeInput {
    match zone.kind {
        ZoneKind::Utc => LocalDateTimeInput {
            value: datetime_local_value(&format_datetime(instant)),
            fold: None,
        },
        ZoneKind::Iana(zone) => {
            let local = instant.with_timezone(&zone);
            let naive = local.naive_local();
            let fold = match zone.from_local_datetime(&naive) {
                LocalResult::Ambiguous(early, late) => {
                    if instant == early.with_timezone(&Utc) {
                        Some(CivilFold::Earlier)
                    } else if instant == late.with_timezone(&Utc) {
                        Some(CivilFold::Later)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            LocalDateTimeInput {
                value: local_clock(naive),
                fold,
            }
        }
    }
}

/// Display stored UTC text in a form. Unknown text is returned unchanged.
pub fn display_datetime_local(stored: &str, zone: ProjectTimeZone) -> (String, Option<CivilFold>) {
    match parse_datetime_utc(stored) {
        Ok(instant) => {
            let input = format_civil_datetime(instant, zone);
            (input.value, input.fold)
        }
        Err(_) => (datetime_local_value(stored), None),
    }
}

fn parse_naive(value: &str) -> Result<NaiveDateTime, &'static str> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M"))
        .map_err(|_| "validation.datetime")
}

fn local_clock(value: NaiveDateTime) -> String {
    if value.nanosecond() == 0 && value.second() == 0 {
        value.format("%Y-%m-%dT%H:%M").to_string()
    } else if value.nanosecond() == 0 {
        value.format("%Y-%m-%dT%H:%M:%S").to_string()
    } else {
        value.format("%Y-%m-%dT%H:%M:%S%.f").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn berlin() -> ProjectTimeZone {
        ProjectTimeZone::parse("Europe/Berlin").unwrap()
    }

    #[test]
    fn utc_zone_keeps_the_historical_local_contract() {
        let zone = ProjectTimeZone::utc();
        let instant = parse_civil_datetime("2027-02-08T09:00", zone, None).unwrap();
        assert_eq!(
            format_datetime(instant),
            format_datetime(parse_datetime_local("2027-02-08T09:00").unwrap())
        );
        assert_eq!(format_datetime(instant), "2027-02-08T09:00:00Z");
        assert_eq!(
            parse_civil_datetime("2027-02-08T09:00Z", zone, None),
            Err("validation.datetime_offset")
        );
        assert_eq!(
            parse_form_datetime("2027-02-08T09:00:00Z", zone, None).map(format_datetime),
            Ok("2027-02-08T09:00:00Z".to_owned())
        );
        assert!(ProjectTimeZone::parse("UTC").unwrap().is_utc());
        assert!(ProjectTimeZone::parse("Etc/UTC").unwrap().is_utc());
        assert_eq!(
            ProjectTimeZone::parse("Europe/Berln"),
            Err("validation.timezone")
        );
        assert_eq!(
            ProjectTimeZone::parse("europe/berlin"),
            Err("validation.timezone")
        );
    }

    #[test]
    fn berlin_winter_and_summer_are_distinct_utc_instants() {
        let zone = berlin();
        let winter = parse_civil_datetime("2027-02-08T09:00", zone, None).unwrap();
        let summer = parse_civil_datetime("2026-07-08T09:00", zone, None).unwrap();
        assert_eq!(format_datetime(winter), "2027-02-08T08:00:00Z");
        assert_eq!(format_datetime(summer), "2026-07-08T07:00:00Z");
        assert_eq!(
            format_civil_datetime(winter, zone).value,
            "2027-02-08T09:00"
        );
        assert_eq!(format_civil_datetime(summer, zone).fold, None);
        let again =
            parse_civil_datetime(&format_civil_datetime(winter, zone).value, zone, None).unwrap();
        assert_eq!(again, winter);
    }

    #[test]
    fn berlin_dst_gap_and_overlap_are_explicit() {
        let zone = berlin();
        assert_eq!(
            parse_civil_datetime("2026-03-29T02:30", zone, None),
            Err("validation.datetime_gap")
        );
        assert_eq!(
            parse_civil_datetime("2026-03-29T02:30", zone, Some(CivilFold::Earlier)),
            Err("validation.datetime_gap")
        );
        let before = parse_civil_datetime("2026-03-29T01:30", zone, None).unwrap();
        let after = parse_civil_datetime("2026-03-29T03:30", zone, None).unwrap();
        assert_eq!(format_datetime(before), "2026-03-29T00:30:00Z");
        assert_eq!(format_datetime(after), "2026-03-29T01:30:00Z");

        assert_eq!(
            parse_civil_datetime("2026-10-25T02:30", zone, None),
            Err("validation.datetime_ambiguous")
        );
        let earlier =
            parse_civil_datetime("2026-10-25T02:30", zone, Some(CivilFold::Earlier)).unwrap();
        let later = parse_civil_datetime("2026-10-25T02:30", zone, Some(CivilFold::Later)).unwrap();
        assert_eq!(format_datetime(earlier), "2026-10-25T00:30:00Z");
        assert_eq!(format_datetime(later), "2026-10-25T01:30:00Z");
        assert_ne!(earlier, later);
        let shown = format_civil_datetime(later, zone);
        assert_eq!(shown.value, "2026-10-25T02:30");
        assert_eq!(shown.fold, Some(CivilFold::Later));
        let round = parse_civil_datetime(&shown.value, zone, shown.fold).unwrap();
        assert_eq!(round, later);
        assert_eq!(
            parse_civil_datetime("2026-07-08T09:00", zone, Some(CivilFold::Earlier)),
            Err("validation.datetime_fold")
        );
    }

    #[test]
    fn transition_days_have_twenty_three_and_twenty_five_hours() {
        let zone = berlin();
        let spring_start = parse_civil_datetime("2026-03-29T00:00", zone, None).unwrap();
        let spring_end = parse_civil_datetime("2026-03-30T00:00", zone, None).unwrap();
        assert_eq!((spring_end - spring_start).num_hours(), 23);
        let autumn_start = parse_civil_datetime("2026-10-25T00:00", zone, None).unwrap();
        let autumn_end = parse_civil_datetime("2026-10-26T00:00", zone, None).unwrap();
        assert_eq!((autumn_end - autumn_start).num_hours(), 25);
    }

    #[test]
    fn invalid_civil_values_keep_stable_codes() {
        let zone = berlin();
        assert_eq!(
            parse_civil_datetime("", zone, None),
            Err("validation.required")
        );
        assert_eq!(
            parse_civil_datetime("2026-02-31T10:00", zone, None),
            Err("validation.datetime")
        );
        assert_eq!(
            parse_civil_datetime("2026-01-01T24:00", zone, None),
            Err("validation.datetime")
        );
        assert_eq!(
            parse_civil_datetime("2026-07-08T09:00+02:00", zone, None),
            Err("validation.datetime_offset")
        );
        assert_eq!(CivilFold::parse("first"), Err("validation.datetime_fold"));
        let new_york = ProjectTimeZone::parse("America/New_York").unwrap();
        let noon = parse_civil_datetime("2026-01-15T12:00", new_york, None).unwrap();
        assert_eq!(format_datetime(noon), "2026-01-15T17:00:00Z");
    }

    #[test]
    fn configured_zone_does_not_read_local_digits_as_utc() {
        let zone = berlin();
        assert_eq!(
            parse_filter_datetime("2027-02-08T09:00", zone).map(format_datetime),
            Ok("2027-02-08T08:00:00Z".to_owned())
        );
        assert_eq!(
            parse_filter_datetime("2027-02-08T09:00:00Z", zone).map(format_datetime),
            Ok("2027-02-08T09:00:00Z".to_owned())
        );
        assert_eq!(
            parse_filter_datetime("2027-02-08T09:00", ProjectTimeZone::utc()),
            Err("validation.datetime")
        );
    }
}
