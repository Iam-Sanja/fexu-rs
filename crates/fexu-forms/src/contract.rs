//! Explicit form contracts. A form lists its own fields; it does not copy a model.

use fexu_core::{parse_bool, parse_i64, text_code};

use crate::{
    bind_submission, BoundField, Field, FieldOverride, FormErrors, FormValues, ModelField,
    SubmitMode, Widget,
};

/// Situation for one submission. Flags are application names such as `"staff"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormContext<'a> {
    pub mode: SubmitMode,
    pub flags: &'a [&'a str],
}

impl<'a> FormContext<'a> {
    pub const fn new(mode: SubmitMode) -> Self {
        Self { mode, flags: &[] }
    }

    pub const fn create() -> Self {
        Self::new(SubmitMode::Create)
    }

    pub fn has(self, flag: &str) -> bool {
        self.flags.contains(&flag)
    }
}

/// Field check used by standalone forms. The error is a stable code.
pub type FieldCheck = fn(&str) -> Result<(), &'static str>;

/// Rust shape of one declared input. This is not a database column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Text,
    OptionalText,
    Integer,
    OptionalInteger,
    Boolean,
    OptionalBoolean,
    Texts,
}

/// One field a form chose to accept.
#[derive(Debug, Clone, Copy)]
pub struct DeclaredField {
    pub name: &'static str,
    pub label: &'static str,
    pub widget: Widget,
    pub help: &'static str,
    pub placeholder: &'static str,
    pub kind: InputKind,
    pub force_required: bool,
    pub read_only: bool,
    pub disabled: bool,
    pub readonly_unless: Option<&'static str>,
    pub required_when: Option<&'static str>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub choices: &'static [&'static str],
    pub check: Option<FieldCheck>,
    pub class_name: &'static str,
}

impl DeclaredField {
    pub const fn new(name: &'static str, label: &'static str, kind: InputKind) -> Self {
        Self {
            name,
            label,
            widget: Widget::Text,
            help: "",
            placeholder: "",
            kind,
            force_required: false,
            read_only: false,
            disabled: false,
            readonly_unless: None,
            required_when: None,
            min_length: None,
            max_length: None,
            choices: &[],
            check: None,
            class_name: "",
        }
    }
}

/// Cleaned value. `Absent` means the key was omitted on a patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanValue {
    Absent,
    Empty,
    Text(String),
    Integer(i64),
    Boolean(bool),
    Texts(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanField {
    pub name: &'static str,
    pub value: CleanValue,
}

/// Context rule for one allow-listed model column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldAccess {
    pub name: &'static str,
    pub readonly_unless: Option<&'static str>,
    pub required_when: Option<&'static str>,
}

/// Unbound values are the initial display. A submitted form shows only the posted values.
#[derive(Debug, Clone, Copy)]
pub struct Presentation<'a> {
    pub initial: &'a FormValues,
    pub submitted: Option<&'a FormValues>,
}

impl<'a> Presentation<'a> {
    pub const fn unbound(initial: &'a FormValues) -> Self {
        Self {
            initial,
            submitted: None,
        }
    }

    pub const fn bound(initial: &'a FormValues, submitted: &'a FormValues) -> Self {
        Self {
            initial,
            submitted: Some(submitted),
        }
    }

    pub const fn is_bound(self) -> bool {
        self.submitted.is_some()
    }

    pub fn display_all(self, name: &str) -> Vec<&'a str> {
        match self.submitted {
            Some(values) => values.get_all(name),
            None => self.initial.get_all(name),
        }
    }
}

/// Posted token for the handler. This crate does not decide that the token is authentic.
pub fn posted_csrf(values: &FormValues) -> Option<&str> {
    values
        .get("csrf_token")
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Create requires every required allow-listed field. Names in `protected` cannot be posted.
pub fn bind_create(
    fields: &[ModelField],
    values: &FormValues,
    protected: &[&str],
) -> Result<Vec<BoundField>, FormErrors> {
    bind_submission(SubmitMode::Create, fields, values, &[], protected)
}

/// Update keeps an omitted allow-listed field unchanged. A protected posted name is rejected.
pub fn bind_update(
    fields: &[ModelField],
    values: &FormValues,
    protected: &[&str],
) -> Result<Vec<BoundField>, FormErrors> {
    bind_submission(SubmitMode::Patch, fields, values, &[], protected)
}

/// Model binding plus flag rules. A locked posted value is `validation.readonly` and is not stored.
pub fn bind_in_context(
    mode: SubmitMode,
    fields: &[ModelField],
    values: &FormValues,
    choices: &[(&str, &[&str])],
    protected: &[&str],
    access: &[FieldAccess],
    context: &FormContext<'_>,
) -> Result<Vec<BoundField>, FormErrors> {
    let mut adjusted = fields.to_vec();
    let mut locked = Vec::new();
    let mut protected_names: Vec<&str> = protected.to_vec();
    for rule in access {
        if rule.readonly_unless.is_some_and(|flag| !context.has(flag)) {
            locked.push(rule.name);
            protected_names.push(rule.name);
        }
        if let Some(flag) = rule.required_when {
            if let Some(field) = adjusted.iter_mut().find(|field| field.name == rule.name) {
                field.required = context.has(flag);
            }
        }
    }
    let posted_locked: Vec<&str> = locked
        .iter()
        .copied()
        .filter(|name| values.get(name).is_some())
        .collect();
    match bind_submission(mode, &adjusted, values, choices, &protected_names) {
        Ok(bound) => Ok(bound),
        Err(mut errors) => {
            errors.fields.retain(|failure| {
                !(failure.code == "validation.required"
                    && posted_locked.iter().any(|name| *name == failure.field))
            });
            Err(errors)
        }
    }
}

/// Validates the declared fields. Unknown names, including a forged column, are rejected.
pub fn bind_declared(
    fields: &[DeclaredField],
    values: &FormValues,
    context: &FormContext<'_>,
) -> Result<Vec<CleanField>, FormErrors> {
    let mut errors = FormErrors::default();
    for name in values.names() {
        if name != "csrf_token" && !fields.iter().any(|field| field.name == name) {
            errors.push(name, "validation.unknown_field");
        }
    }
    let mut clean = Vec::with_capacity(fields.len());
    for field in fields {
        match clean_one(field, values, context) {
            Ok(value) => clean.push(CleanField {
                name: field.name,
                value,
            }),
            Err(code) => errors.push(field.name, code),
        }
    }
    if errors.is_empty() {
        Ok(clean)
    } else {
        Err(errors)
    }
}

pub fn render_presentation(
    action: &str,
    csrf: Option<&str>,
    presentation: Presentation<'_>,
    errors: &FormErrors,
    fields: &[Field],
    locale: &str,
    overrides: &[FieldOverride],
) -> String {
    let mut shown = FormValues::default();
    for field in fields {
        shown.set_all(field.name, presentation.display_all(field.name));
    }
    crate::render_fields_with(action, csrf, &shown, errors, fields, locale, overrides)
}

fn clean_one(
    field: &DeclaredField,
    values: &FormValues,
    context: &FormContext<'_>,
) -> Result<CleanValue, &'static str> {
    let locked = field.disabled
        || field.read_only
        || field.readonly_unless.is_some_and(|flag| !context.has(flag));
    let present = values.get(field.name).is_some();
    if locked {
        return if present {
            Err("validation.readonly")
        } else {
            Ok(CleanValue::Absent)
        };
    }
    if !field_multiple(field) && values.get_all(field.name).len() > 1 {
        return Err("validation.duplicate");
    }
    let required = field
        .required_when
        .map(|flag| context.has(flag))
        .unwrap_or_else(|| {
            field.force_required
                || matches!(
                    field.kind,
                    InputKind::Text | InputKind::Integer | InputKind::Boolean
                )
        });
    if field.kind == InputKind::Texts {
        return clean_texts(field, values, context.mode, required);
    }
    let raw = values.get(field.name).unwrap_or("").trim();
    if !present || raw.is_empty() {
        if context.mode == SubmitMode::Patch && !present {
            return Ok(CleanValue::Absent);
        }
        return if required {
            Err("validation.required")
        } else {
            Ok(CleanValue::Empty)
        };
    }
    if !field.choices.is_empty() && !field.choices.contains(&raw) {
        return Err("validation.choice");
    }
    let value = match field.kind {
        InputKind::Text | InputKind::OptionalText => {
            check_text(field, raw)?;
            CleanValue::Text(raw.to_owned())
        }
        InputKind::Integer | InputKind::OptionalInteger => {
            CleanValue::Integer(parse_i64(raw, None, None)?)
        }
        InputKind::Boolean | InputKind::OptionalBoolean => CleanValue::Boolean(parse_bool(raw)?),
        InputKind::Texts => unreachable!("texts are handled above"),
    };
    Ok(value)
}

fn clean_texts(
    field: &DeclaredField,
    values: &FormValues,
    mode: SubmitMode,
    required: bool,
) -> Result<CleanValue, &'static str> {
    let present = values.get(field.name).is_some();
    let items = values.get_all(field.name);
    if !present || items.iter().all(|item| item.trim().is_empty()) {
        if mode == SubmitMode::Patch && !present {
            return Ok(CleanValue::Absent);
        }
        return if required {
            Err("validation.required")
        } else {
            Ok(CleanValue::Texts(Vec::new()))
        };
    }
    let mut clean = Vec::new();
    for item in items {
        let item = item.trim();
        if item.is_empty() {
            return Err("validation.required");
        }
        if !field.choices.is_empty() && !field.choices.contains(&item) {
            return Err("validation.choice");
        }
        check_text(field, item)?;
        clean.push(item.to_owned());
    }
    Ok(CleanValue::Texts(clean))
}

fn check_text(field: &DeclaredField, raw: &str) -> Result<(), &'static str> {
    let email = field.widget == Widget::Email;
    let url = field.widget == Widget::Url;
    if let Some(code) = text_code(raw, true, field.min_length, field.max_length, email, url) {
        return Err(code);
    }
    if let Some(check) = field.check {
        check(raw)?;
    }
    Ok(())
}

fn field_multiple(field: &DeclaredField) -> bool {
    field.kind == InputKind::Texts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model_fields, BoundValue};
    use fexu_core::{AdminFieldMeta, ColumnKind, ColumnSpec};

    fn title_column() -> ColumnSpec {
        ColumnSpec {
            name: "title",
            kind: ColumnKind::Text,
            nullable: false,
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
    fn repeated_keys_stay_errors_unless_the_field_is_multiple() {
        let duplicate = FormValues::parse_urlencoded("tag=a&tag=b").unwrap_err();
        assert_eq!(duplicate.for_field("tag"), Some("validation.duplicate"));
        let mixed = FormValues::parse_urlencoded_multi("tag=a&tag=b&name=Ada&name=Bea", &["tag"])
            .unwrap_err();
        assert_eq!(mixed.for_field("name"), Some("validation.duplicate"));
        let values = FormValues::parse_urlencoded_multi("tag=a&tag=b&name=Ada", &["tag"]).unwrap();
        assert_eq!(values.get_all("tag"), ["a", "b"]);
        assert_eq!(values.get("name"), Some("Ada"));
        let json =
            FormValues::parse_json_multi(r#"{"tag":["a","b"],"name":"Ada"}"#, &["tag"]).unwrap();
        assert_eq!(json.get_all("tag"), ["a", "b"]);
        let rejected = FormValues::parse_json(r#"{"tag":["a"]}"#).unwrap_err();
        assert_eq!(rejected.for_field("tag"), Some("validation.invalid_type"));
    }

    #[test]
    fn declared_form_rejects_unknown_readonly_and_cross_field_codes() {
        fn reserved(value: &str) -> Result<(), &'static str> {
            if value == "admin" {
                Err("validation.choice")
            } else {
                Ok(())
            }
        }
        let fields = [
            DeclaredField {
                check: Some(reserved),
                min_length: Some(2),
                ..DeclaredField::new("name", "Name", InputKind::Text)
            },
            DeclaredField {
                readonly_unless: Some("staff"),
                ..DeclaredField::new("note", "Note", InputKind::OptionalText)
            },
            DeclaredField {
                choices: &["red", "blue"],
                widget: Widget::SelectMultiple,
                ..DeclaredField::new("tags", "Tags", InputKind::Texts)
            },
        ];
        let mut posted = FormValues::parse_urlencoded_multi(
            "name=Ada&note=secret&tags=red&tags=blue&owner_id=1",
            &["tags"],
        )
        .unwrap();
        let errors = bind_declared(&fields, &posted, &FormContext::create()).unwrap_err();
        assert_eq!(
            errors.for_field("owner_id"),
            Some("validation.unknown_field")
        );
        assert_eq!(errors.for_field("note"), Some("validation.readonly"));
        assert!(errors.field_errors().any(|failure| failure.field == "note"));
        posted.remove("owner_id");
        posted.remove("note");
        posted.insert("name", "admin");
        assert_eq!(
            bind_declared(&fields, &posted, &FormContext::create())
                .unwrap_err()
                .for_field("name"),
            Some("validation.choice")
        );
        posted.insert("name", "Ada");
        let clean = bind_declared(
            &fields,
            &posted,
            &FormContext {
                mode: SubmitMode::Create,
                flags: &["staff"],
            },
        )
        .unwrap();
        assert_eq!(clean[0].value, CleanValue::Text("Ada".into()));
        assert_eq!(
            clean[2].value,
            CleanValue::Texts(vec!["red".into(), "blue".into()])
        );
        let mut mismatch = FormErrors::default();
        mismatch.push_form("validation.mismatch");
        assert_eq!(mismatch.form_error(), Some("validation.mismatch"));
        assert_eq!(mismatch.field_errors().count(), 0);
    }

    #[test]
    fn create_and_update_contracts_keep_protected_columns_out() {
        let mut secret = title_column();
        secret.name = "secret";
        secret.nullable = true;
        let title = title_column();
        let admin = [AdminFieldMeta {
            name: "secret",
            label: "Secret",
            list_display: false,
            list_link: false,
            searchable: false,
            sortable: false,
            filterable: false,
            readonly: false,
            hidden: false,
            sensitive: true,
            fieldset: "",
        }];
        assert_eq!(
            model_fields(&[title, secret], &admin, &["secret"])
                .unwrap_err()
                .code,
            "form.protected_column"
        );
        let fields = model_fields(&[title], &[], &["title"]).unwrap();
        let mut forged = FormValues::default();
        forged.insert("title", "Visible");
        forged.insert("secret", "nope");
        let errors = bind_create(&fields, &forged, &["secret"]).unwrap_err();
        assert_eq!(errors.for_field("secret"), Some("validation.readonly"));
        forged.remove("secret");
        let created = bind_create(&fields, &forged, &["secret"]).unwrap();
        assert_eq!(created[0].value, BoundValue::Text("Visible".into()));
        let patched = bind_update(&fields, &FormValues::default(), &["secret"]).unwrap();
        assert_eq!(patched[0].value, BoundValue::Unchanged);
        let access = [FieldAccess {
            name: "title",
            readonly_unless: Some("staff"),
            required_when: None,
        }];
        let mut staff_post = FormValues::default();
        staff_post.insert("title", "Changed");
        let denied = bind_in_context(
            SubmitMode::Patch,
            &fields,
            &staff_post,
            &[],
            &[],
            &access,
            &FormContext::new(SubmitMode::Patch),
        )
        .unwrap_err();
        assert_eq!(denied.for_field("title"), Some("validation.readonly"));
        let allowed = bind_in_context(
            SubmitMode::Patch,
            &fields,
            &staff_post,
            &[],
            &[],
            &access,
            &FormContext {
                mode: SubmitMode::Patch,
                flags: &["staff"],
            },
        )
        .unwrap();
        assert_eq!(allowed[0].value, BoundValue::Text("Changed".into()));
    }

    #[test]
    fn bound_presentation_does_not_fall_back_to_initial_values() {
        let mut initial = FormValues::default();
        initial.insert("title", "Draft");
        let mut submitted = FormValues::default();
        submitted.insert("title", "Posted");
        let unbound = Presentation::unbound(&initial);
        assert_eq!(unbound.display_all("title"), ["Draft"]);
        assert!(!unbound.is_bound());
        let bound = Presentation::bound(&initial, &submitted);
        assert_eq!(bound.display_all("title"), ["Posted"]);
        assert!(bound.display_all("missing").is_empty());
        assert!(posted_csrf(&submitted).is_none());
        submitted.insert("csrf_token", "posted-token");
        assert_eq!(posted_csrf(&submitted), Some("posted-token"));
    }
}
