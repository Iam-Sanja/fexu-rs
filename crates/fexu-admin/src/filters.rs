//! Registered list filters. Column definitions come from the registry; values are bound.

use fexu_core::{AdminModelMeta, ColumnKind, ColumnSpec};
use fexu_db::driver::Value as DbValue;

use crate::views::{valid_key, valid_label, ExtensionError};

#[derive(Clone, Debug)]
pub struct FilterOption {
    pub value: &'static str,
    pub label_de: &'static str,
    pub label_en: &'static str,
}

impl FilterOption {
    pub const fn new(value: &'static str, german: &'static str, english: &'static str) -> Self {
        Self {
            value,
            label_de: german,
            label_en: english,
        }
    }
}

#[derive(Clone, Debug)]
pub enum FilterKind {
    Equals {
        column: &'static str,
    },
    Choice {
        column: &'static str,
        options: &'static [FilterOption],
    },
    /// Inclusive `from` and `to` dates on a `Date` or `DateTime` column.
    ///
    /// The query keys are `filter.{name}.from` and `filter.{name}.to`. Either
    /// edge may be omitted. A datetime column uses UTC midnight, matching the
    /// date hierarchy, and the `to` date stays included through the next midnight.
    DateRange {
        column: &'static str,
    },
}

#[derive(Clone, Debug)]
pub struct AdminFilter {
    pub model_key: &'static str,
    pub name: &'static str,
    pub label_de: &'static str,
    pub label_en: &'static str,
    pub kind: FilterKind,
}

impl AdminFilter {
    pub fn equals(model_key: &'static str, name: &'static str, column: &'static str) -> Self {
        Self {
            model_key,
            name,
            label_de: name,
            label_en: name,
            kind: FilterKind::Equals { column },
        }
    }

    pub fn choice(
        model_key: &'static str,
        name: &'static str,
        column: &'static str,
        options: &'static [FilterOption],
    ) -> Self {
        Self {
            model_key,
            name,
            label_de: name,
            label_en: name,
            kind: FilterKind::Choice { column, options },
        }
    }

    /// Inclusive date window. `column` must be `Date` or `DateTime`.
    pub fn date_range(model_key: &'static str, name: &'static str, column: &'static str) -> Self {
        Self {
            model_key,
            name,
            label_de: name,
            label_en: name,
            kind: FilterKind::DateRange { column },
        }
    }

    pub fn labels(mut self, german: &'static str, english: &'static str) -> Self {
        self.label_de = german;
        self.label_en = english;
        self
    }

    pub(crate) fn label(&self, locale: &str) -> &'static str {
        if locale == "de" {
            self.label_de
        } else {
            self.label_en
        }
    }

    pub(crate) fn column(&self) -> &'static str {
        match self.kind {
            FilterKind::Equals { column }
            | FilterKind::Choice { column, .. }
            | FilterKind::DateRange { column } => column,
        }
    }

    pub(crate) fn bind(
        &self,
        columns: &[ColumnSpec],
        value: &str,
    ) -> Result<(&'static str, DbValue), String> {
        let column = columns
            .iter()
            .find(|column| column.name == self.column())
            .ok_or_else(|| "filter field is not allowed".to_owned())?;
        if value.is_empty() {
            return Err("filter field is not allowed".into());
        }
        match &self.kind {
            FilterKind::Choice { options, .. } => {
                if !options.iter().any(|option| option.value == value) {
                    return Err("filter value is not allowed".into());
                }
            }
            FilterKind::Equals { .. } => {}
            FilterKind::DateRange { .. } => {
                return Err("filter range uses from and to".into());
            }
        }
        typed(column, value).map(|bound| (column.name, bound))
    }
}

pub(crate) fn check_filter(
    filter: &AdminFilter,
    meta: Option<&AdminModelMeta>,
    columns: Option<&[ColumnSpec]>,
) -> Result<(), ExtensionError> {
    if !valid_key(filter.model_key) || !valid_key(filter.name) {
        return Err(ExtensionError::new(
            "extension.filter",
            "Filter name or model name is invalid.",
        ));
    }
    if !valid_label(filter.label_de) || !valid_label(filter.label_en) {
        return Err(ExtensionError::new(
            "extension.filter",
            "The filter label is empty or contains control characters.",
        ));
    }
    if !valid_key(filter.column()) {
        return Err(ExtensionError::new(
            "extension.filter",
            "Die Filterspalte ist kein Modellfeld.",
        ));
    }
    match &filter.kind {
        FilterKind::Choice { options, .. } => {
            if options.is_empty() || options.len() > 20 {
                return Err(ExtensionError::new(
                    "extension.filter",
                    "Eine Auswahl braucht zwischen 1 und 20 feste Werte.",
                ));
            }
            if options.iter().any(|option| {
                option.value.is_empty()
                    || option.value.len() > 80
                    || !valid_label(option.label_de)
                    || !valid_label(option.label_en)
                    || option.value.contains('&')
                    || option.value.contains('=')
            }) {
                return Err(ExtensionError::new(
                    "extension.filter",
                    "A choice value is empty or contains control characters.",
                ));
            }
        }
        FilterKind::Equals { .. } | FilterKind::DateRange { .. } => {}
    }
    if matches!(filter.kind, FilterKind::DateRange { .. }) {
        if let Some(columns) = columns {
            let Some(column) = columns.iter().find(|column| column.name == filter.column()) else {
                return Err(ExtensionError::new(
                    "extension.filter",
                    "The filter column does not belong to the model.",
                ));
            };
            if !matches!(column.kind, ColumnKind::Date | ColumnKind::DateTime) {
                return Err(ExtensionError::new(
                    "extension.filter",
                    "A date range filter needs a date or datetime column.",
                ));
            }
        }
    }
    if let (Some(meta), Some(columns)) = (meta, columns) {
        if meta.fields.iter().any(|field| field.name == filter.name) {
            return Err(ExtensionError::new(
                "extension.filter",
                "Der Filtername kollidiert mit einem Modellfeld.",
            ));
        }
        let Some(column) = columns.iter().find(|column| column.name == filter.column()) else {
            return Err(ExtensionError::new(
                "extension.filter",
                "The filter column does not belong to the model.",
            ));
        };
        if column.primary_key {
            return Err(ExtensionError::new(
                "extension.filter",
                "A filter cannot target the primary key.",
            ));
        }
        let hidden = meta
            .fields
            .iter()
            .any(|field| field.name == column.name && (field.hidden || field.sensitive));
        if hidden {
            return Err(ExtensionError::new(
                "extension.filter",
                "A filter cannot expose a hidden field.",
            ));
        }
    }
    Ok(())
}

fn typed(column: &ColumnSpec, value: &str) -> Result<DbValue, String> {
    match column.kind {
        ColumnKind::Text => Ok(DbValue::from(value.to_owned())),
        ColumnKind::BigInt => value
            .parse::<i64>()
            .map(DbValue::from)
            .map_err(|_| format!("filter {} must be an integer", column.name)),
        ColumnKind::Boolean => match value {
            "true" | "1" => Ok(DbValue::from(true)),
            "false" | "0" => Ok(DbValue::from(false)),
            _ => Err(format!("filter {} must be true or false", column.name)),
        },
        ColumnKind::Uuid => uuid::Uuid::parse_str(value)
            .map(DbValue::from)
            .map_err(|_| format!("filter {} must be a uuid", column.name)),
        ColumnKind::Date => fexu_core::parse_date(value)
            .map(DbValue::from)
            .map_err(|_| format!("filter {} must be a date", column.name)),
        ColumnKind::Time => fexu_core::parse_time(value)
            .map(DbValue::from)
            .map_err(|_| format!("filter {} must be a time", column.name)),
        ColumnKind::DateTime => filter_datetime(column, value),
        ColumnKind::Decimal => decimal_filter(column, value),
        ColumnKind::Choice => choice_filter(column, value),
        ColumnKind::Json => Err("json columns have no portable filter".into()),
    }
}

fn filter_datetime(column: &ColumnSpec, value: &str) -> Result<DbValue, String> {
    let zone = fexu_core::project_zone_from_env()
        .map_err(|_| format!("filter {} {}", column.name, "validation.timezone"))?;
    fexu_core::parse_filter_datetime(value, zone)
        .map(DbValue::from)
        .map_err(|code| format!("filter {} {code}", column.name))
}

fn decimal_filter(column: &ColumnSpec, value: &str) -> Result<DbValue, String> {
    let parsed = fexu_core::parse_decimal(value, column.precision, column.scale)
        .map_err(|_| format!("filter {} must be a decimal", column.name))?;
    let bytes = fexu_core::encode_decimal(parsed, column.precision, column.scale)
        .map_err(|_| format!("filter {} must be a decimal", column.name))?;
    Ok(DbValue::Bytes(Some(Box::new(bytes.to_vec()))))
}

fn choice_filter(column: &ColumnSpec, value: &str) -> Result<DbValue, String> {
    if !fexu_core::choice_is_known(column.choices, value) {
        return Err(format!("filter {} is not a known choice", column.name));
    }
    Ok(DbValue::from(value.to_owned()))
}

/// Inclusive date bounds. `to` becomes an exclusive upper bound on the next day.
pub(crate) fn date_range_bounds(
    column: &ColumnSpec,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<(&'static str, bool, DbValue)>, String> {
    if !matches!(column.kind, ColumnKind::Date | ColumnKind::DateTime) {
        return Err(format!("filter {} must be a date", column.name));
    }
    let start = match from {
        Some(value) => Some(parse_range_date(column, value)?),
        None => None,
    };
    let end = match to {
        Some(value) => Some(parse_range_date(column, value)?),
        None => None,
    };
    if let (Some(start), Some(end)) = (start, end) {
        if start > end {
            return Err(format!("filter {} range is reversed", column.name));
        }
    }
    let mut bounds = Vec::new();
    if let Some(start) = start {
        bounds.push((column.name, true, range_value(column, start)?));
    }
    if let Some(end) = end {
        let next = end
            .succ_opt()
            .ok_or_else(|| format!("filter {} must be a date", column.name))?;
        bounds.push((column.name, false, range_value(column, next)?));
    }
    Ok(bounds)
}

fn parse_range_date(column: &ColumnSpec, value: &str) -> Result<chrono::NaiveDate, String> {
    fexu_core::parse_date(value).map_err(|_| format!("filter {} must be a date", column.name))
}

fn range_value(column: &ColumnSpec, date: chrono::NaiveDate) -> Result<DbValue, String> {
    match column.kind {
        ColumnKind::Date => Ok(DbValue::from(date)),
        ColumnKind::DateTime => {
            let instant = date
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| format!("filter {} must be a date", column.name))?
                .and_utc();
            Ok(DbValue::from(instant))
        }
        _ => Err(format!("filter {} must be a date", column.name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date_column() -> ColumnSpec {
        ColumnSpec {
            name: "due",
            kind: ColumnKind::Date,
            nullable: true,
            primary_key: false,
            default_bool: None,
            unique: false,
            indexed: false,
            foreign_table: None,
            foreign_column: None,
            on_delete: None,
            precision: 0,
            scale: 0,
            choices: &[],
        }
    }

    #[test]
    fn date_range_includes_both_ends() {
        let column = date_column();
        let bounds = date_range_bounds(&column, Some("2026-11-01"), Some("2026-11-03")).unwrap();
        assert_eq!(bounds.len(), 2);
        assert_eq!(bounds[0].0, "due");
        assert!(bounds[0].1);
        assert_eq!(
            bounds[0].2,
            DbValue::from(fexu_core::parse_date("2026-11-01").unwrap())
        );
        assert!(!bounds[1].1);
        assert_eq!(
            bounds[1].2,
            DbValue::from(fexu_core::parse_date("2026-11-04").unwrap())
        );
    }

    #[test]
    fn reversed_date_range_is_rejected() {
        let error =
            date_range_bounds(&date_column(), Some("2026-11-03"), Some("2026-11-01")).unwrap_err();
        assert!(error.contains("reversed"));
    }
}
