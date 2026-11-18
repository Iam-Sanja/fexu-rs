//! Stable error order and constraint codes.
//!
//! Field failures stay in declaration order. A form-level code stays on the empty
//! name and is reported before field codes. Constraint helpers copy only the column
//! names the caller already knows from the model. They do not read a database error
//! string into the page.

use crate::{valid_field_name, FieldFailure, FormErrors};

/// Which database constraint the caller already classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintKind {
    Unique,
    ForeignKey,
}

/// Field or form codes for one constraint. An empty column list is form-level.
///
/// A column that is not a field name, or a repeated column, is rejected and no
/// partial error list is returned. The database message is not copied.
pub fn constraint_errors(
    kind: ConstraintKind,
    columns: &[&str],
) -> Result<FormErrors, &'static str> {
    let code = constraint_code(kind);
    let mut errors = FormErrors::default();
    if columns.is_empty() {
        errors.push_form(code);
        return Ok(errors);
    }
    for column in columns {
        if !valid_field_name(column) {
            return Err("validation.invalid_type");
        }
        if errors.for_field(column).is_some() {
            return Err("validation.duplicate");
        }
        errors.push(*column, code);
    }
    Ok(errors)
}

/// Maps the two exact repository messages. Any other text returns `Ok(None)`.
pub fn constraint_errors_from_message(
    message: &str,
    columns: &[&str],
) -> Result<Option<FormErrors>, &'static str> {
    let kind = match message {
        "unique constraint violated" => ConstraintKind::Unique,
        "foreign key constraint violated" => ConstraintKind::ForeignKey,
        _ => return Ok(None),
    };
    constraint_errors(kind, columns).map(Some)
}

fn constraint_code(kind: ConstraintKind) -> &'static str {
    match kind {
        ConstraintKind::Unique => "validation.unique",
        ConstraintKind::ForeignKey => "validation.relation_denied",
    }
}

/// Form-level codes first, then `fields` in the given order, then any other field.
pub fn validation_order<'a>(errors: &'a FormErrors, fields: &[&str]) -> Vec<&'a FieldFailure> {
    let mut ordered = Vec::new();
    for failure in &errors.fields {
        if failure.field.is_empty() {
            ordered.push(failure);
        }
    }
    for name in fields {
        for failure in &errors.fields {
            if failure.field == *name {
                ordered.push(failure);
            }
        }
    }
    for failure in &errors.fields {
        if !failure.field.is_empty() && !fields.contains(&failure.field.as_str()) {
            ordered.push(failure);
        }
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraint_codes_do_not_copy_the_database_text() {
        let unique = constraint_errors(ConstraintKind::Unique, &["sku"]).unwrap();
        assert_eq!(unique.for_field("sku"), Some("validation.unique"));
        assert!(constraint_errors(ConstraintKind::Unique, &["sku sku"]).is_err());
        assert!(constraint_errors(ConstraintKind::Unique, &["sku", "sku"]).is_err());
        let form = constraint_errors_from_message("unique constraint violated", &[]).unwrap();
        assert_eq!(form.unwrap().form_error(), Some("validation.unique"));
        let foreign =
            constraint_errors_from_message("foreign key constraint violated", &["lead_id"])
                .unwrap()
                .unwrap();
        assert_eq!(
            foreign.for_field("lead_id"),
            Some("validation.relation_denied")
        );
        assert!(
            constraint_errors_from_message("duplicate key value (sku)", &["sku"])
                .unwrap()
                .is_none()
        );
        let mut errors = FormErrors::default();
        errors.push("email", "validation.email");
        errors.push_form("validation.mismatch");
        errors.push("name", "validation.min_length");
        errors.push("role", "validation.unknown_field");
        let ordered = validation_order(&errors, &["name", "email"]);
        let codes: Vec<_> = ordered
            .iter()
            .map(|failure| (failure.field.as_str(), failure.code))
            .collect();
        assert_eq!(
            codes,
            vec![
                ("", "validation.mismatch"),
                ("name", "validation.min_length"),
                ("email", "validation.email"),
                ("role", "validation.unknown_field"),
            ]
        );
    }
}
