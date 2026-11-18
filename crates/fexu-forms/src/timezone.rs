//! Form binding for civil `datetime-local` values.
//!
//! `required_datetime_local` stays the UTC contract. These helpers take an
//! explicit [`ProjectTimeZone`]. `bind_model` still expects a canonical UTC
//! string for a `DateTime` column: convert first, then store `format_datetime`.

use chrono::{DateTime, Utc};
use fexu_core::civil::{format_civil_datetime, parse_civil_datetime, CivilFold, ProjectTimeZone};
use fexu_core::temporal::format_datetime;

use crate::{escape, FieldFailure, FormValues};

pub use fexu_core::civil::{CivilFold as Fold, LocalDateTimeInput, ProjectTimeZone as Zone};

/// Companion control. HTML and JSON both post this name when the civil time is ambiguous.
pub fn datetime_fold_name(field: &str) -> String {
    format!("{field}__fold")
}

pub fn required_datetime_in_zone(
    values: &FormValues,
    name: &'static str,
    zone: ProjectTimeZone,
) -> Result<DateTime<Utc>, FieldFailure> {
    let value = values.get(name).unwrap_or("");
    if value.trim().is_empty() {
        return Err(FieldFailure {
            field: name.to_owned(),
            code: "validation.required",
        });
    }
    let fold = read_fold(values, name)?;
    parse_civil_datetime(value, zone, fold).map_err(|code| FieldFailure {
        field: name.to_owned(),
        code,
    })
}

pub fn optional_datetime_in_zone(
    values: &FormValues,
    name: &'static str,
    zone: ProjectTimeZone,
) -> Result<Option<DateTime<Utc>>, FieldFailure> {
    let value = values.get(name).unwrap_or("");
    if value.trim().is_empty() {
        return Ok(None);
    }
    required_datetime_in_zone(values, name, zone).map(Some)
}

/// Input plus a hidden fold control when the rendered instant is ambiguous.
pub fn render_datetime_local(name: &str, instant: DateTime<Utc>, zone: ProjectTimeZone) -> String {
    let input = format_civil_datetime(instant, zone);
    let mut html = format!(
        "<input type=\"datetime-local\" id=\"{name}\" name=\"{name}\" value=\"{value}\">",
        name = escape(name),
        value = escape(&input.value)
    );
    if let Some(fold) = input.fold {
        let fold_name = datetime_fold_name(name);
        html.push_str(&format!(
            "<input type=\"hidden\" id=\"{id}\" name=\"{fold_name}\" value=\"{fold}\">",
            id = escape(&format!("{name}-fold")),
            fold_name = escape(&fold_name),
            fold = escape(fold.as_str())
        ));
    }
    html
}

/// Canonical UTC text for `bind_model` after a zoned parse.
pub fn canonical_utc(instant: DateTime<Utc>) -> String {
    format_datetime(instant)
}

fn read_fold(values: &FormValues, name: &str) -> Result<Option<CivilFold>, FieldFailure> {
    match values
        .get(&datetime_fold_name(name))
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => Ok(None),
        Some(value) => CivilFold::parse(value)
            .map(Some)
            .map_err(|code| FieldFailure {
                field: datetime_fold_name(name),
                code,
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{message, required_datetime_local, FormValues};
    use fexu_core::temporal::format_datetime;

    fn berlin() -> ProjectTimeZone {
        ProjectTimeZone::parse("Europe/Berlin").unwrap()
    }

    #[test]
    fn html_and_json_roundtrip_winter_summer_and_overlap() {
        let zone = berlin();
        let winter = required_from_json(r#"{"starts":"2027-02-08T09:00"}"#, zone);
        assert_eq!(format_datetime(winter), "2027-02-08T08:00:00Z");
        let html = render_datetime_local("starts", winter, zone);
        assert!(html.contains("value=\"2027-02-08T09:00\""));
        assert!(!html.contains("__fold"));
        let summer = required_from_form("starts=2026-07-08T09:00", zone);
        assert_eq!(format_datetime(summer), "2026-07-08T07:00:00Z");
        assert_eq!(canonical_utc(summer), "2026-07-08T07:00:00Z");

        let mut values = FormValues::default();
        values.insert("starts", "2026-10-25T02:30");
        assert_eq!(
            required_datetime_in_zone(&values, "starts", zone)
                .unwrap_err()
                .code,
            "validation.datetime_ambiguous"
        );
        values.insert("starts__fold", "later");
        let later = required_datetime_in_zone(&values, "starts", zone).unwrap();
        let rendered = render_datetime_local("starts", later, zone);
        assert!(rendered.contains("name=\"starts__fold\""));
        assert!(rendered.contains("value=\"later\""));
        let parsed =
            FormValues::parse_urlencoded("starts=2026-10-25T02%3A30&starts__fold=later").unwrap();
        assert_eq!(
            required_datetime_in_zone(&parsed, "starts", zone).unwrap(),
            later
        );
    }

    #[test]
    fn gap_invalid_and_utc_compatibility_share_codes() {
        let zone = berlin();
        let mut values = FormValues::default();
        values.insert("starts", "2026-03-29T02:30");
        assert_eq!(
            required_datetime_in_zone(&values, "starts", zone)
                .unwrap_err()
                .code,
            "validation.datetime_gap"
        );
        values.insert("starts", "2026-02-31T10:00");
        assert_eq!(
            required_datetime_in_zone(&values, "starts", zone)
                .unwrap_err()
                .code,
            "validation.datetime"
        );
        values.insert("starts", "2027-02-08T09:00");
        let zoned = required_datetime_in_zone(&values, "starts", zone).unwrap();
        let utc = required_datetime_local(&values, "starts").unwrap();
        assert_ne!(zoned, utc);
        assert_eq!(format_datetime(utc), "2027-02-08T09:00:00Z");
        assert_eq!(
            message("en", "validation.datetime_gap"),
            "That local time does not exist because of a daylight-saving change."
        );
        assert_eq!(
            message("de", "validation.datetime_ambiguous"),
            "Diese Ortszeit gibt es zweimal. earlier oder later ist erforderlich."
        );
        let json = FormValues::parse_json(r#"{"starts":"2026-03-29T02:30"}"#).unwrap();
        assert_eq!(
            required_datetime_in_zone(&json, "starts", zone)
                .unwrap_err()
                .code,
            "validation.datetime_gap"
        );
    }

    fn required_from_json(body: &str, zone: ProjectTimeZone) -> chrono::DateTime<Utc> {
        let values = FormValues::parse_json(body).unwrap();
        required_datetime_in_zone(&values, "starts", zone).unwrap()
    }

    fn required_from_form(body: &str, zone: ProjectTimeZone) -> chrono::DateTime<Utc> {
        let values = FormValues::parse_urlencoded(body).unwrap();
        required_datetime_in_zone(&values, "starts", zone).unwrap()
    }
}
