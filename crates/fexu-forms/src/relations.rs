//! Relation fields. The caller supplies the ids already limited by `QueryScope`.
//!
//! This module does not query the database and does not widen that scope.
//! A posted id is accepted only when it is in `allowed`.

use uuid::Uuid;

use crate::{escape, valid_field_name, FormErrors, FormValues};

/// One page of a relation widget. Larger sets stay on the next page.
pub const RELATION_PAGE_LIMIT: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationLink {
    ForeignKey,
    OneToOne,
    ManyToMany,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationPresentation {
    Select,
    SelectMultiple,
    Search,
    Picker,
    Readonly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationOption {
    pub id: Uuid,
    pub label: String,
}

/// Ids the handler already checked. `allowed` is the selectable scope.
/// `known` distinguishes a missing id from one that exists outside the scope.
/// Pass the same ids for both when the caller must not reveal that difference.
#[derive(Debug, Clone, Copy)]
pub struct RelationInput<'a> {
    pub name: &'a str,
    pub link: RelationLink,
    pub required: bool,
    pub readonly: bool,
    pub allowed: &'a [Uuid],
    pub known: &'a [Uuid],
    pub locked: &'a [Uuid],
}

#[derive(Debug, Clone, Copy)]
pub struct RelationWidget<'a> {
    pub name: &'a str,
    pub label: &'a str,
    pub presentation: RelationPresentation,
    pub required: bool,
    pub options: &'a [RelationOption],
    pub selected: &'a [Uuid],
    pub query: &'a str,
    pub has_more: bool,
    pub next_query: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundRelation {
    One(Option<Uuid>),
    Many(Vec<Uuid>),
}

/// Returns at most [`RELATION_PAGE_LIMIT`] options starting at `offset`.
pub fn relation_window(options: &[RelationOption], offset: usize) -> &[RelationOption] {
    let Some(rest) = options.get(offset..) else {
        return &[];
    };
    let end = rest.len().min(RELATION_PAGE_LIMIT);
    &rest[..end]
}

/// Accepts one foreign key, one one-to-one target, or a many-to-many selection.
pub fn bind_relation(
    values: &FormValues,
    input: RelationInput<'_>,
) -> Result<BoundRelation, FormErrors> {
    let mut errors = FormErrors::default();
    if !valid_field_name(input.name) {
        errors.push(input.name, "validation.invalid_type");
        return Err(errors);
    }
    let posted = posted_ids(values, input.name, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    if input.readonly && !same_ids(&posted, input.locked) {
        errors.push(input.name, "validation.readonly");
        return Err(errors);
    }
    if posted.len() > RELATION_PAGE_LIMIT {
        errors.push(input.name, "form.relation_page");
        return Err(errors);
    }
    let multiple = input.link == RelationLink::ManyToMany;
    if !multiple && posted.len() > 1 {
        errors.push(input.name, "validation.duplicate");
        return Err(errors);
    }
    if multiple && has_duplicate(&posted) {
        errors.push(input.name, "validation.duplicate");
        return Err(errors);
    }
    if posted.is_empty() {
        if input.required {
            errors.push(input.name, "validation.required");
            return Err(errors);
        }
        return Ok(if multiple {
            BoundRelation::Many(Vec::new())
        } else {
            BoundRelation::One(None)
        });
    }
    for id in &posted {
        if input.allowed.contains(id) {
            continue;
        }
        let code = if input.known.contains(id) {
            "validation.relation_denied"
        } else {
            "validation.relation_missing"
        };
        errors.push(input.name, code);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(if multiple {
        BoundRelation::Many(posted)
    } else {
        BoundRelation::One(posted.first().copied())
    })
}

/// Renders only the supplied page. More than [`RELATION_PAGE_LIMIT`] options is an error.
pub fn render_relation_widget(widget: RelationWidget<'_>) -> Result<String, &'static str> {
    if !valid_field_name(widget.name)
        || widget.label.is_empty()
        || widget.options.len() > RELATION_PAGE_LIMIT
    {
        return Err("form.relation_page");
    }
    if let Some(query) = widget.next_query {
        if !safe_next_query(query) {
            return Err("form.relation_page");
        }
    }
    let name = escape(widget.name);
    let label = escape(widget.label);
    let mut html = format!("<label for=\"{name}\">{label}</label>");
    match widget.presentation {
        RelationPresentation::Readonly => {
            html.push_str(&readonly_markup(&name, widget.options, widget.selected));
        }
        RelationPresentation::Search | RelationPresentation::Picker => {
            let query = escape(widget.query);
            html.push_str(&format!(
                "<input id=\"{name}__query\" name=\"{name}__query\" type=\"search\" value=\"{query}\">"
            ));
            html.push_str(&select_options(widget));
            if widget.has_more {
                if let Some(next) = widget.next_query {
                    let href = escape(next);
                    html.push_str(&format!("<a href=\"?{href}\">Continue searching</a>"));
                }
            }
        }
        RelationPresentation::Select | RelationPresentation::SelectMultiple => {
            html.push_str(&select_options(widget));
        }
    }
    Ok(html)
}

fn posted_ids(values: &FormValues, name: &str, errors: &mut FormErrors) -> Vec<Uuid> {
    let mut ids = Vec::new();
    for raw in values.get_all(name) {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        match Uuid::parse_str(raw) {
            Ok(id) => ids.push(id),
            Err(_) => errors.push(name, "validation.relation"),
        }
    }
    ids
}

fn same_ids(left: &[Uuid], right: &[Uuid]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().all(|id| right.contains(id)) && right.iter().all(|id| left.contains(id))
}

fn has_duplicate(ids: &[Uuid]) -> bool {
    ids.iter()
        .enumerate()
        .any(|(index, id)| ids[..index].contains(id))
}

fn select_options(widget: RelationWidget<'_>) -> String {
    let name = escape(widget.name);
    let multiple = if widget.presentation == RelationPresentation::SelectMultiple {
        " multiple"
    } else {
        ""
    };
    let required = if widget.required && multiple.is_empty() {
        " required"
    } else {
        ""
    };
    let mut html = format!(
        "<select id=\"{name}\" name=\"{name}\"{multiple}{required} data-fexu-relation=\"true\">"
    );
    if required.is_empty() && multiple.is_empty() {
        html.push_str("<option value=\"\"></option>");
    }
    for option in widget.options {
        let selected = if widget.selected.contains(&option.id) {
            " selected"
        } else {
            ""
        };
        html.push_str(&format!(
            "<option value=\"{}\"{selected}>{}</option>",
            escape(&option.id.to_string()),
            escape(&option.label)
        ));
    }
    html.push_str("</select>");
    html
}

fn readonly_markup(name: &str, options: &[RelationOption], selected: &[Uuid]) -> String {
    let mut html = String::new();
    for id in selected {
        let Some(option) = options.iter().find(|option| option.id == *id) else {
            continue;
        };
        html.push_str(&format!(
            "<span>{}</span><input type=\"hidden\" name=\"{name}\" value=\"{}\">",
            escape(&option.label),
            escape(&option.id.to_string())
        ));
    }
    html
}

fn safe_next_query(query: &str) -> bool {
    !query.is_empty()
        && query.len() <= 200
        && !query.contains(':')
        && !query.contains('/')
        && !query.contains('\\')
        && !query.contains('<')
        && !query.contains('>')
        && !query.contains('"')
        && !query.contains('\'')
        && query.chars().all(|char| {
            char.is_ascii_alphanumeric() || matches!(char, '?' | '&' | '=' | '_' | '%' | '.' | '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(id: u128, label: &str) -> RelationOption {
        RelationOption {
            id: Uuid::from_u128(id),
            label: label.to_owned(),
        }
    }

    #[test]
    fn relation_binding_rejects_forged_missing_and_foreign_ids() {
        let allowed = [Uuid::from_u128(1)];
        let known = [Uuid::from_u128(1), Uuid::from_u128(2)];
        let input = RelationInput {
            name: "lead_id",
            link: RelationLink::ForeignKey,
            required: true,
            readonly: false,
            allowed: &allowed,
            known: &known,
            locked: &[],
        };
        let mut values = FormValues::default();
        values.insert("lead_id", Uuid::from_u128(1).to_string());
        assert_eq!(
            bind_relation(&values, input).unwrap(),
            BoundRelation::One(Some(Uuid::from_u128(1)))
        );
        values.insert("lead_id", Uuid::from_u128(2).to_string());
        assert_eq!(
            bind_relation(&values, input)
                .unwrap_err()
                .for_field("lead_id"),
            Some("validation.relation_denied")
        );
        values.insert("lead_id", Uuid::from_u128(3).to_string());
        assert_eq!(
            bind_relation(&values, input)
                .unwrap_err()
                .for_field("lead_id"),
            Some("validation.relation_missing")
        );
        values.insert("lead_id", "not-an-id");
        assert_eq!(
            bind_relation(&values, input)
                .unwrap_err()
                .for_field("lead_id"),
            Some("validation.relation")
        );
        values.insert("lead_id", "");
        assert_eq!(
            bind_relation(&values, input)
                .unwrap_err()
                .for_field("lead_id"),
            Some("validation.required")
        );
        let optional = RelationInput {
            required: false,
            ..input
        };
        assert_eq!(
            bind_relation(&values, optional).unwrap(),
            BoundRelation::One(None)
        );
    }

    #[test]
    fn one_to_one_and_many_to_many_keep_their_cardinality() {
        let allowed = [Uuid::from_u128(1), Uuid::from_u128(2)];
        let mut values = FormValues::default();
        values.set_all(
            "desk_id",
            [
                Uuid::from_u128(1).to_string(),
                Uuid::from_u128(2).to_string(),
            ],
        );
        let one = RelationInput {
            name: "desk_id",
            link: RelationLink::OneToOne,
            required: false,
            readonly: false,
            allowed: &allowed,
            known: &allowed,
            locked: &[],
        };
        assert_eq!(
            bind_relation(&values, one)
                .unwrap_err()
                .for_field("desk_id"),
            Some("validation.duplicate")
        );
        let many = RelationInput {
            name: "desk_id",
            link: RelationLink::ManyToMany,
            ..one
        };
        assert_eq!(
            bind_relation(&values, many).unwrap(),
            BoundRelation::Many(vec![Uuid::from_u128(1), Uuid::from_u128(2)])
        );
        values.set_all(
            "desk_id",
            [
                Uuid::from_u128(1).to_string(),
                Uuid::from_u128(1).to_string(),
            ],
        );
        assert_eq!(
            bind_relation(&values, many)
                .unwrap_err()
                .for_field("desk_id"),
            Some("validation.duplicate")
        );
        let locked = [Uuid::from_u128(1)];
        values.insert("desk_id", Uuid::from_u128(2).to_string());
        let readonly = RelationInput {
            name: "desk_id",
            link: RelationLink::ForeignKey,
            required: true,
            readonly: true,
            allowed: &allowed,
            known: &allowed,
            locked: &locked,
        };
        assert_eq!(
            bind_relation(&values, readonly)
                .unwrap_err()
                .for_field("desk_id"),
            Some("validation.readonly")
        );
    }

    #[test]
    fn widgets_render_a_page_and_refuse_a_thousand_rows() {
        let none = render_relation_widget(RelationWidget {
            name: "lead_id",
            label: "Lead",
            presentation: RelationPresentation::Select,
            required: false,
            options: &[],
            selected: &[],
            query: "",
            has_more: false,
            next_query: None,
        })
        .unwrap();
        assert!(none.contains("<option value=\"\"></option>"));
        assert!(!none.contains("<script>"));
        let one = [option(1, "<Ada>")];
        let selected = [Uuid::from_u128(1)];
        let html = render_relation_widget(RelationWidget {
            name: "lead_id",
            label: "Lead",
            presentation: RelationPresentation::Search,
            required: true,
            options: &one,
            selected: &selected,
            query: "ad",
            has_more: true,
            next_query: Some("lead_id__page=2"),
        })
        .unwrap();
        assert!(html.contains("&lt;Ada&gt;"));
        assert!(html.contains("type=\"search\""));
        assert!(html.contains("selected"));
        assert!(html.contains("Continue searching"));
        assert!(!html.contains("<Ada>"));
        let hundred: Vec<_> = (1..=100).map(|id| option(id, "row")).collect();
        let page = render_relation_widget(RelationWidget {
            name: "lead_id",
            label: "Lead",
            presentation: RelationPresentation::Picker,
            required: false,
            options: &hundred,
            selected: &[],
            query: "",
            has_more: false,
            next_query: None,
        })
        .unwrap();
        assert_eq!(page.matches("<option value=\"").count(), 101);
        let thousand: Vec<_> = (1..=1000).map(|id| option(id, "row")).collect();
        assert_eq!(relation_window(&thousand, 0).len(), 100);
        assert_eq!(relation_window(&thousand, 950).len(), 50);
        assert!(relation_window(&thousand, 1000).is_empty());
        let allowed: Vec<_> = thousand.iter().map(|option| option.id).collect();
        let mut values = FormValues::default();
        values.insert("lead_id", allowed[500].to_string());
        let bound = bind_relation(
            &values,
            RelationInput {
                name: "lead_id",
                link: RelationLink::ForeignKey,
                required: true,
                readonly: false,
                allowed: &allowed,
                known: &allowed,
                locked: &[],
            },
        )
        .unwrap();
        assert_eq!(bound, BoundRelation::One(Some(allowed[500])));
        assert!(render_relation_widget(RelationWidget {
            name: "lead_id",
            label: "Lead",
            presentation: RelationPresentation::Select,
            required: false,
            options: &thousand,
            selected: &[],
            query: "",
            has_more: true,
            next_query: Some("https://evil.example"),
        })
        .is_err());
    }
}
