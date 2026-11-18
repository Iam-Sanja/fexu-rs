//! Model and inline formsets on top of the existing `{prefix}_{index}_{field}` names.
//!
//! `bind_formset` is unchanged. These helpers add management fields, deletion,
//! optional order, a minimum size, and a parent id that the client cannot replace.

use uuid::Uuid;

use crate::{escape, valid_field_name, FormErrors, FormValues, SubmitMode, FORMSET_MAX_ROWS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFormset {
    pub prefix: &'static str,
    pub min_rows: usize,
    pub max_rows: usize,
    pub can_delete: bool,
    pub can_order: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct InlineFormset<'a> {
    pub formset: ModelFormset,
    pub parent_field: &'static str,
    pub parent_id: Uuid,
    pub owned_ids: &'a [Uuid],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineChange<T> {
    pub index: usize,
    pub order: Option<i64>,
    pub action: InlineAction<T>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineAction<T> {
    Create(T),
    Update { id: Uuid, value: T },
    Delete { id: Uuid },
}

pub fn bind_model_formset<T>(
    values: &FormValues,
    config: ModelFormset,
    mode: SubmitMode,
    parse: impl FnMut(SubmitMode, &FormValues) -> Result<T, FormErrors>,
    cross: impl FnMut(&[InlineChange<T>]) -> Result<(), FormErrors>,
) -> Result<Vec<InlineChange<T>>, FormErrors> {
    bind_rows(values, config, None, mode, parse, cross)
}

pub fn bind_inline_formset<T>(
    values: &FormValues,
    inline: InlineFormset<'_>,
    mode: SubmitMode,
    parse: impl FnMut(SubmitMode, &FormValues) -> Result<T, FormErrors>,
    cross: impl FnMut(&[InlineChange<T>]) -> Result<(), FormErrors>,
) -> Result<Vec<InlineChange<T>>, FormErrors> {
    bind_rows(values, inline.formset, Some(inline), mode, parse, cross)
}

/// Hidden management inputs. Row values stay in the posted `FormValues`.
pub fn render_formset_management(config: ModelFormset, total: usize, initial: usize) -> String {
    let prefix = escape(config.prefix);
    format!(
        "<input type=\"hidden\" name=\"{prefix}_TOTAL_FORMS\" value=\"{total}\"><input type=\"hidden\" name=\"{prefix}_INITIAL_FORMS\" value=\"{initial}\"><input type=\"hidden\" name=\"{prefix}_MIN_FORMS\" value=\"{}\"><input type=\"hidden\" name=\"{prefix}_MAX_FORMS\" value=\"{}\">",
        config.min_rows, config.max_rows
    )
}

fn bind_rows<T>(
    values: &FormValues,
    config: ModelFormset,
    inline: Option<InlineFormset<'_>>,
    mode: SubmitMode,
    mut parse: impl FnMut(SubmitMode, &FormValues) -> Result<T, FormErrors>,
    mut cross: impl FnMut(&[InlineChange<T>]) -> Result<(), FormErrors>,
) -> Result<Vec<InlineChange<T>>, FormErrors> {
    let mut errors = FormErrors::default();
    if !valid_field_name(config.prefix)
        || config.max_rows == 0
        || config.max_rows > FORMSET_MAX_ROWS
        || config.min_rows > config.max_rows
    {
        errors.push(config.prefix, "form.formset");
        return Err(errors);
    }
    let total_name = format!("{}_TOTAL_FORMS", config.prefix);
    let initial_name = format!("{}_INITIAL_FORMS", config.prefix);
    let min_name = format!("{}_MIN_FORMS", config.prefix);
    let max_name = format!("{}_MAX_FORMS", config.prefix);
    let total = match values.get(&total_name).map(str::trim) {
        Some(raw) => match raw.parse::<usize>() {
            Ok(total) if total <= config.max_rows => total,
            Ok(_) => {
                errors.push(&total_name, "validation.formset_size");
                return Err(errors);
            }
            Err(_) => {
                errors.push(&total_name, "validation.invalid_type");
                return Err(errors);
            }
        },
        None => {
            errors.push(&total_name, "validation.required");
            return Err(errors);
        }
    };
    if let Some(raw) = values.get(&initial_name).map(str::trim) {
        match raw.parse::<usize>() {
            Ok(initial) if initial <= total => {}
            Ok(_) => errors.push(&initial_name, "validation.formset_size"),
            Err(_) => errors.push(&initial_name, "validation.invalid_type"),
        }
    }
    if let Some(raw) = values.get(&min_name).map(str::trim) {
        if raw.parse::<usize>().ok() != Some(config.min_rows) {
            errors.push(&min_name, "validation.formset_size");
        }
    }
    if let Some(raw) = values.get(&max_name).map(str::trim) {
        if raw.parse::<usize>().ok() != Some(config.max_rows) {
            errors.push(&max_name, "validation.formset_size");
        }
    }
    let marker = format!("{}_", config.prefix);
    for name in values.names() {
        if name == total_name || name == initial_name || name == min_name || name == max_name {
            continue;
        }
        if !name.starts_with(&marker) {
            continue;
        }
        let rest = &name[marker.len()..];
        let Some((index, _)) = rest.split_once('_') else {
            errors.push(name, "validation.unknown_field");
            continue;
        };
        match index.parse::<usize>() {
            Ok(index) if index < total => {}
            _ => errors.push(name, "validation.formset_size"),
        }
    }
    let mut rows = Vec::new();
    let mut seen_orders = Vec::new();
    let mut kept = 0usize;
    for index in 0..total {
        if let Some(change) = read_row(values, config, inline, mode, index, &mut parse, &mut errors)
        {
            if !matches!(change.action, InlineAction::Delete { .. }) {
                kept += 1;
                if let Some(order) = change.order {
                    if seen_orders.contains(&order) {
                        errors.push(
                            format!("{}_{index}_ORDER", config.prefix),
                            "validation.formset_order",
                        );
                    }
                    seen_orders.push(order);
                }
            }
            rows.push(change);
        }
    }
    if kept < config.min_rows {
        errors.push(&total_name, "validation.formset_size");
    }
    if errors.is_empty() {
        if let Err(mut cross_errors) = cross(&rows) {
            errors.fields.append(&mut cross_errors.fields);
        }
    }
    if errors.is_empty() {
        Ok(rows)
    } else {
        Err(errors)
    }
}

fn read_row<T>(
    values: &FormValues,
    config: ModelFormset,
    inline: Option<InlineFormset<'_>>,
    mode: SubmitMode,
    index: usize,
    parse: &mut impl FnMut(SubmitMode, &FormValues) -> Result<T, FormErrors>,
    errors: &mut FormErrors,
) -> Option<InlineChange<T>> {
    let prefix = format!("{}_{index}_", config.prefix);
    let id_name = format!("{prefix}id");
    let delete_name = format!("{prefix}DELETE");
    let order_name = format!("{prefix}ORDER");
    let id = match values
        .get(&id_name)
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
    {
        Some(raw) => match Uuid::parse_str(raw) {
            Ok(id) => Some(id),
            Err(_) => {
                errors.push(&id_name, "validation.relation");
                None
            }
        },
        None => None,
    };
    let delete = match values.get(&delete_name).map(str::trim) {
        Some("1") => true,
        Some("") | None => false,
        Some(_) => {
            errors.push(&delete_name, "validation.invalid_type");
            false
        }
    };
    if delete && !config.can_delete {
        errors.push(&delete_name, "validation.readonly");
    }
    let order = if config.can_order {
        match values
            .get(&order_name)
            .map(str::trim)
            .filter(|raw| !raw.is_empty())
        {
            Some(raw) => match raw.parse::<i64>() {
                Ok(order) => Some(order),
                Err(_) => {
                    errors.push(&order_name, "validation.invalid_type");
                    None
                }
            },
            None => None,
        }
    } else if values.get(&order_name).is_some() {
        errors.push(&order_name, "validation.unknown_field");
        None
    } else {
        None
    };
    if let Some(parent) = inline {
        let parent_name = format!("{prefix}{}", parent.parent_field);
        if let Some(raw) = values.get(&parent_name).map(str::trim) {
            if raw != parent.parent_id.to_string() {
                errors.push(&parent_name, "validation.relation_denied");
            }
        }
        if let Some(id) = id {
            if !parent.owned_ids.contains(&id) {
                errors.push(&id_name, "validation.relation_denied");
            }
        }
    }
    if delete {
        return match id {
            Some(id)
                if errors.for_field(&id_name).is_none()
                    && errors.for_field(&delete_name).is_none() =>
            {
                Some(InlineChange {
                    index,
                    order,
                    action: InlineAction::Delete { id },
                })
            }
            _ => None,
        };
    }
    let mut row = FormValues::default();
    for name in values.names() {
        let Some(field) = name.strip_prefix(&prefix) else {
            continue;
        };
        if field == "id" || field == "DELETE" || field == "ORDER" {
            continue;
        }
        if inline.is_some_and(|parent| field == parent.parent_field) {
            continue;
        }
        if let Some(value) = values.get(name) {
            row.insert(field, value);
        }
    }
    match parse(mode, &row) {
        Ok(value) => Some(InlineChange {
            index,
            order,
            action: match id {
                Some(id) => InlineAction::Update { id, value },
                None => InlineAction::Create(value),
            },
        }),
        Err(row_errors) => {
            for failure in row_errors.fields {
                let field = if failure.field.is_empty() {
                    prefix.trim_end_matches('_').to_owned()
                } else {
                    format!("{prefix}{}", failure.field)
                };
                errors.push(field, failure.code);
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ModelFormset {
        ModelFormset {
            prefix: "tasks",
            min_rows: 1,
            max_rows: 5,
            can_delete: true,
            can_order: true,
        }
    }

    fn title(mode: SubmitMode, row: &FormValues) -> Result<String, FormErrors> {
        let _ = mode;
        match row.get("title").map(str::trim) {
            Some(title) if !title.is_empty() => Ok(title.to_owned()),
            _ => {
                let mut errors = FormErrors::default();
                errors.push("title", "validation.required");
                Err(errors)
            }
        }
    }

    #[test]
    fn model_formset_checks_management_fields_order_and_keeps_posted_values() {
        let mut values = FormValues::default();
        values.insert("tasks_TOTAL_FORMS", "2");
        values.insert("tasks_INITIAL_FORMS", "1");
        values.insert("tasks_MIN_FORMS", "0");
        values.insert("tasks_0_title", "Keep");
        values.insert("tasks_0_ORDER", "1");
        values.insert("tasks_1_title", "");
        values.insert("tasks_1_ORDER", "1");
        values.insert("tasks_2_title", "forged");
        let errors = bind_model_formset(&values, config(), SubmitMode::Create, title, |_| Ok(()))
            .unwrap_err();
        assert_eq!(
            errors.for_field("tasks_MIN_FORMS"),
            Some("validation.formset_size")
        );
        assert_eq!(
            errors.for_field("tasks_1_title"),
            Some("validation.required")
        );
        assert_eq!(
            errors.for_field("tasks_2_title"),
            Some("validation.formset_size")
        );
        assert_eq!(values.get("tasks_0_title"), Some("Keep"));
        let duplicate =
            FormValues::parse_urlencoded("tasks_TOTAL_FORMS=1&tasks_TOTAL_FORMS=2&tasks_0_title=A")
                .unwrap_err();
        assert_eq!(
            duplicate.for_field("tasks_TOTAL_FORMS"),
            Some("validation.duplicate")
        );
        let mut valid = FormValues::default();
        valid.insert("tasks_TOTAL_FORMS", "2");
        valid.insert("tasks_0_id", Uuid::from_u128(7).to_string());
        valid.insert("tasks_0_title", "Plan");
        valid.insert("tasks_0_ORDER", "1");
        valid.insert("tasks_1_title", "Build");
        valid.insert("tasks_1_ORDER", "2");
        let rows = bind_model_formset(&valid, config(), SubmitMode::Create, title, |rows| {
            let titles: Vec<_> = rows
                .iter()
                .filter_map(|row| match &row.action {
                    InlineAction::Create(title) | InlineAction::Update { value: title, .. } => {
                        Some(title.as_str())
                    }
                    InlineAction::Delete { .. } => None,
                })
                .collect();
            if titles.len()
                != titles
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
            {
                let mut errors = FormErrors::default();
                errors.push_form("validation.duplicate");
                return Err(errors);
            }
            Ok(())
        })
        .unwrap();
        assert!(matches!(rows[0].action, InlineAction::Update { .. }));
        assert!(matches!(rows[1].action, InlineAction::Create(_)));
        valid.insert("tasks_1_title", "Plan");
        assert_eq!(
            bind_model_formset(&valid, config(), SubmitMode::Create, title, |rows| {
                let mut seen = Vec::new();
                for row in rows {
                    let (InlineAction::Create(title) | InlineAction::Update { value: title, .. }) =
                        &row.action
                    else {
                        continue;
                    };
                    if seen.contains(&title.as_str()) {
                        let mut errors = FormErrors::default();
                        errors.push_form("validation.duplicate");
                        return Err(errors);
                    }
                    seen.push(title.as_str());
                }
                Ok(())
            })
            .unwrap_err()
            .form_error(),
            Some("validation.duplicate")
        );
    }

    #[test]
    fn inline_formset_rejects_a_foreign_parent_and_a_foreign_child() {
        let parent = Uuid::from_u128(4);
        let owned = [Uuid::from_u128(8)];
        let inline = InlineFormset {
            formset: config(),
            parent_field: "project_id",
            parent_id: parent,
            owned_ids: &owned,
        };
        let mut values = FormValues::default();
        values.insert("tasks_TOTAL_FORMS", "2");
        values.insert("tasks_0_id", Uuid::from_u128(8).to_string());
        values.insert("tasks_0_title", "Own");
        values.insert("tasks_0_project_id", parent.to_string());
        values.insert("tasks_1_id", Uuid::from_u128(9).to_string());
        values.insert("tasks_1_title", "Stolen");
        values.insert("tasks_1_project_id", Uuid::from_u128(5).to_string());
        let errors = bind_inline_formset(&values, inline, SubmitMode::Create, title, |_| Ok(()))
            .unwrap_err();
        assert_eq!(
            errors.for_field("tasks_1_id"),
            Some("validation.relation_denied")
        );
        assert_eq!(
            errors.for_field("tasks_1_project_id"),
            Some("validation.relation_denied")
        );
        assert_eq!(values.get("tasks_0_title"), Some("Own"));
        values.remove("tasks_1_id");
        values.remove("tasks_1_project_id");
        values.insert("tasks_1_title", "Next");
        let rows =
            bind_inline_formset(&values, inline, SubmitMode::Create, title, |_| Ok(())).unwrap();
        assert!(matches!(
            rows[0].action,
            InlineAction::Update { id, .. } if id == owned[0]
        ));
        assert!(matches!(rows[1].action, InlineAction::Create(_)));
        values.insert("tasks_0_DELETE", "1");
        values.insert("tasks_1_DELETE", "1");
        let deleted = bind_inline_formset(&values, inline, SubmitMode::Create, title, |_| Ok(()))
            .unwrap_err();
        assert_eq!(
            deleted.for_field("tasks_TOTAL_FORMS"),
            Some("validation.formset_size")
        );
        let html = render_formset_management(config(), 1, 1);
        assert!(html.contains("type=\"hidden\""));
        assert!(html.contains("tasks_TOTAL_FORMS"));
    }
}
