//! Renders a relation widget from a scope-limited lookup page.
//!
//! The page comes from [`crate::lookup_page`]. This module does not run a
//! second query and does not add ids that were outside that page.

use fexu_forms::{
    render_relation_widget, RelationOption, RelationPresentation, RelationWidget,
    RELATION_PAGE_LIMIT,
};
use uuid::Uuid;

use crate::relations::LookupPage;

pub fn options_from_lookup(
    page: &LookupPage,
    label: &str,
) -> Result<Vec<RelationOption>, &'static str> {
    if page.rows.len() > RELATION_PAGE_LIMIT {
        return Err("form.relation_page");
    }
    let mut options = Vec::with_capacity(page.rows.len());
    for row in &page.rows {
        let id = row
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(|raw| Uuid::parse_str(raw).ok())
            .ok_or("validation.relation")?;
        let text = row
            .get(label)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        options.push(RelationOption {
            id,
            label: text.to_owned(),
        });
    }
    Ok(options)
}

pub fn render_scoped_page(
    name: &str,
    label: &str,
    column: &str,
    presentation: RelationPresentation,
    page: &LookupPage,
    selected: &[Uuid],
    query: &str,
) -> Result<String, &'static str> {
    let options = options_from_lookup(page, column)?;
    render_relation_widget(RelationWidget {
        name,
        label,
        presentation,
        required: false,
        options: &options,
        selected,
        query,
        has_more: page.has_more,
        next_query: None,
    })
}
