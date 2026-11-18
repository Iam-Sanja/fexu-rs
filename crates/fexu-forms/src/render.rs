//! Form and field rendering. Fragments passed to a template are already escaped.
//!
//! Field order is the slice order. [`order_fields`] reorders a copy. A prefix is
//! applied to the HTML name and id only; [`unprefix`] restores the declared names
//! before binding. `csrf_token` is not prefixed.

use fexu_core::{message_for, FieldOverride};

use crate::{
    escape, message, render_control, valid_field_name, Field, FormErrors, FormValues, Widget,
};

/// Pieces of one form. Each string is finished HTML.
#[derive(Debug, Clone, Copy)]
pub struct FormFragments<'a> {
    pub start: &'a str,
    pub summary: &'a str,
    pub controls: &'a str,
    pub submit: &'a str,
    pub end: &'a str,
}

/// One field, already escaped and split so a template can reorder the pieces.
#[derive(Debug, Clone, Copy)]
pub struct FieldFragments<'a> {
    pub label: &'a str,
    pub control: &'a str,
    pub error: &'a str,
    pub help: &'a str,
}

pub type FormTemplate = fn(FormFragments<'_>) -> String;
pub type FieldTemplate = fn(FieldFragments<'_>) -> String;

/// One render. Empty prefix, class and submit label keep the historical markup.
#[derive(Debug, Clone, Copy)]
pub struct FormRender<'a> {
    pub action: &'a str,
    pub csrf: Option<&'a str>,
    pub values: &'a FormValues,
    pub errors: &'a FormErrors,
    pub fields: &'a [Field],
    pub locale: &'a str,
    pub overrides: &'a [FieldOverride],
    pub prefix: &'a str,
    pub class_name: &'a str,
    pub submit_label: &'a str,
    pub form_template: Option<FormTemplate>,
    pub field_template: Option<FieldTemplate>,
}

impl<'a> FormRender<'a> {
    pub fn new(
        action: &'a str,
        values: &'a FormValues,
        errors: &'a FormErrors,
        fields: &'a [Field],
        locale: &'a str,
    ) -> Self {
        Self {
            action,
            csrf: None,
            values,
            errors,
            fields,
            locale,
            overrides: &[],
            prefix: "",
            class_name: "",
            submit_label: "",
            form_template: None,
            field_template: None,
        }
    }
}

pub fn render_form(spec: FormRender<'_>) -> String {
    if !spec.prefix.is_empty() && !valid_field_name(spec.prefix) {
        return rejected(spec.locale);
    }
    for field in spec.fields {
        if input_name(spec.prefix, field.name).is_none() {
            return rejected(spec.locale);
        }
    }
    let class = if spec.class_name.is_empty() {
        String::new()
    } else {
        format!(" class=\"{}\"", escape(spec.class_name))
    };
    let enctype = if spec.fields.iter().any(|field| field.widget == Widget::File) {
        " enctype=\"multipart/form-data\""
    } else {
        ""
    };
    let start = {
        let mut html = format!(
            "<form method=\"post\" action=\"{}\"{class}{enctype}>",
            escape(spec.action)
        );
        if let Some(token) = spec.csrf {
            html.push_str(&format!(
                "<input type=\"hidden\" name=\"csrf_token\" value=\"{}\">",
                escape(token)
            ));
        }
        html
    };
    let summary = summary(
        spec.errors,
        spec.fields,
        spec.prefix,
        spec.locale,
        spec.overrides,
    );
    let mut controls = String::new();
    let field_template = spec.field_template.unwrap_or(default_field);
    for field in spec.fields {
        let Some(input_name) = input_name(spec.prefix, field.name) else {
            return rejected(spec.locale);
        };
        let shown = displayed(spec.values, spec.prefix, field.name, &input_name);
        let control = render_control(field, &input_name, &shown, spec.errors, spec.locale);
        let id = escape(&input_name);
        let label = if field.widget == Widget::Hidden {
            String::new()
        } else {
            let mut html = format!("<label for=\"{id}\">{label}", label = escape(field.label));
            if field.required {
                html.push_str("<span aria-hidden=\"true\">*</span>");
            }
            html.push_str("</label>");
            html
        };
        let error = if let Some(code) = spec.errors.for_field(field.name) {
            format!(
                "<p id=\"{id}-error\" role=\"alert\">{text}</p>",
                text = escape(&message_for(
                    spec.locale,
                    field.name,
                    code,
                    &[],
                    spec.overrides
                ))
            )
        } else {
            String::new()
        };
        let help = if field.help.is_empty() {
            String::new()
        } else {
            format!("<p id=\"{id}-help\">{text}</p>", text = escape(field.help))
        };
        controls.push_str(&field_template(FieldFragments {
            label: &label,
            control: &control,
            error: &error,
            help: &help,
        }));
    }
    let submit_text = if spec.submit_label.is_empty() {
        if spec.locale == "de" {
            "Senden".to_owned()
        } else {
            "Send".to_owned()
        }
    } else {
        escape(spec.submit_label)
    };
    let submit = format!("<button type=\"submit\">{submit_text}</button>");
    let end = "</form>";
    let paint = spec.form_template.unwrap_or(default_form);
    paint(FormFragments {
        start: &start,
        summary: &summary,
        controls: &controls,
        submit: &submit,
        end,
    })
}

fn default_form(parts: FormFragments<'_>) -> String {
    format!(
        "{}{}{}{}{}",
        parts.start, parts.summary, parts.controls, parts.submit, parts.end
    )
}

fn default_field(parts: FieldFragments<'_>) -> String {
    format!(
        "{}{}{}{}",
        parts.label, parts.control, parts.error, parts.help
    )
}

fn summary(
    errors: &FormErrors,
    fields: &[Field],
    prefix: &str,
    locale: &str,
    overrides: &[FieldOverride],
) -> String {
    let mut html = String::new();
    if let Some(code) = errors.for_field("") {
        html.push_str(&format!(
            "<p role=\"alert\">{}</p>",
            escape(&message_for(locale, "", code, &[], overrides))
        ));
    }
    let mut items = String::new();
    for field in fields {
        let Some(code) = errors.for_field(field.name) else {
            continue;
        };
        let Some(id) = input_name(prefix, field.name) else {
            continue;
        };
        items.push_str(&format!(
            "<li><a href=\"#{id}\">{label}: {text}</a></li>",
            id = escape(&id),
            label = escape(field.label),
            text = escape(&message_for(locale, field.name, code, &[], overrides))
        ));
    }
    for failure in errors.field_errors() {
        if fields.iter().any(|field| field.name == failure.field) {
            continue;
        }
        let label = escape(&failure.field);
        let text = escape(&message_for(
            locale,
            &failure.field,
            failure.code,
            &[],
            overrides,
        ));
        if let Some(id) = input_name(prefix, &failure.field) {
            items.push_str(&format!(
                "<li><a href=\"#{id}\">{label}: {text}</a></li>",
                id = escape(&id)
            ));
        } else {
            items.push_str(&format!("<li>{label}: {text}</li>"));
        }
    }
    if !items.is_empty() {
        html.push_str("<ul>");
        html.push_str(&items);
        html.push_str("</ul>");
    }
    html
}

fn displayed(values: &FormValues, prefix: &str, field_name: &str, input_name: &str) -> Vec<String> {
    let source = if !prefix.is_empty() && values.get(input_name).is_some() {
        input_name
    } else {
        field_name
    };
    values
        .get_all(source)
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn input_name(prefix: &str, name: &str) -> Option<String> {
    let input_name = if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}_{name}")
    };
    valid_field_name(&input_name).then_some(input_name)
}

fn rejected(locale: &str) -> String {
    format!(
        "<p role=\"alert\">{}</p>",
        escape(message(locale, "validation.invalid_type"))
    )
}

/// Restores declared field names. A prefixed and an unprefixed copy of the same name
/// is `validation.duplicate`. `csrf_token` is copied unchanged.
pub fn unprefix(values: &FormValues, prefix: &str) -> Result<FormValues, FormErrors> {
    if prefix.is_empty() {
        return Ok(values.clone());
    }
    if !valid_field_name(prefix) {
        let mut errors = FormErrors::default();
        errors.push_form("validation.invalid_type");
        return Err(errors);
    }
    let marker = format!("{prefix}_");
    let mut next = FormValues::default();
    let mut errors = FormErrors::default();
    for name in values.names() {
        if name == "csrf_token" {
            if let Some(token) = values.get(name) {
                next.insert(name, token);
            }
            continue;
        }
        let bare = name.strip_prefix(&marker).unwrap_or(name);
        if !valid_field_name(bare) {
            errors.push(name, "validation.invalid_type");
            continue;
        }
        if next.get(bare).is_some() {
            errors.push(bare, "validation.duplicate");
            continue;
        }
        let repeated = values.get_all(name);
        next.set_all(bare, repeated.iter().copied());
    }
    if errors.is_empty() {
        Ok(next)
    } else {
        Err(errors)
    }
}

/// Field order for one page. `names` must be a permutation of the slice.
pub fn order_fields(fields: &[Field], names: &[&str]) -> Result<Vec<Field>, FormErrors> {
    let mut errors = FormErrors::default();
    if names.len() != fields.len() {
        errors.push_form("validation.invalid_type");
        return Err(errors);
    }
    let mut ordered = Vec::with_capacity(fields.len());
    for name in names {
        if ordered.iter().any(|field: &Field| field.name == *name) {
            errors.push(*name, "validation.duplicate");
            return Err(errors);
        }
        let Some(field) = fields.iter().find(|field| field.name == *name) else {
            errors.push(*name, "validation.unknown_field");
            return Err(errors);
        };
        ordered.push(*field);
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FieldOverride, Widget};

    fn bad_widget(input: crate::WidgetInput<'_>) -> String {
        format!(
            "<i>{}</i>",
            input.values.first().map(String::as_str).unwrap_or("")
        )
    }

    #[test]
    fn prefix_template_and_summary_escape_request_text() {
        let fields = [
            Field::new("note", "<b>Note</b>", Widget::Text, true).css_class("wide"),
            Field::new("tone", "Tone", Widget::Custom(bad_widget), false).css_class("tone"),
        ];
        let ordered = order_fields(&fields, &["tone", "note"]).unwrap();
        assert_eq!(ordered[0].name, "tone");
        let mut posted = FormValues::default();
        posted.insert("edit_note", "<script>");
        posted.insert("edit_tone", "<script>");
        posted.insert("csrf_token", "token&");
        let mut errors = FormErrors::default();
        errors.push("note", "validation.required");
        errors.push("role", "validation.unknown_field");
        errors.push_form("validation.mismatch");
        let overrides = [FieldOverride {
            field: "note",
            code: "validation.required",
            de: "Pflicht <i>",
            en: "Required <i>",
        }];
        let mut spec = FormRender::new("/save", &posted, &errors, &ordered, "en");
        spec.csrf = Some("token&");
        spec.prefix = "edit";
        spec.class_name = "sheet";
        spec.overrides = &overrides;
        spec.submit_label = "Keep <b>";
        spec.form_template = Some(|parts| {
            format!(
                "<section>{}{}{}{}{}</section>",
                parts.start, parts.summary, parts.controls, parts.submit, parts.end
            )
        });
        let html = render_form(spec);
        assert!(html.starts_with("<section><form"));
        assert!(html.contains("class=\"sheet\""));
        assert!(html.contains("name=\"edit_note\""));
        assert!(html.contains("value=\"&lt;script&gt;\""));
        assert!(html.contains("class=\"wide\""));
        assert!(html.contains("Required &lt;i&gt;"));
        assert!(html.contains("role: This field is not allowed."));
        assert!(html.contains("href=\"#edit_note\""));
        assert!(html.contains("&lt;b&gt;Note&lt;/b&gt;"));
        assert!(html.contains("Keep &lt;b&gt;"));
        assert!(html.contains("value=\"token&amp;\""));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<i>"));
        let bare = unprefix(&posted, "edit").unwrap();
        assert_eq!(bare.get("note"), Some("<script>"));
        assert_eq!(bare.get("csrf_token"), Some("token&"));
        posted.insert("note", "other");
        assert_eq!(
            unprefix(&posted, "edit").unwrap_err().for_field("note"),
            Some("validation.duplicate")
        );
    }
}
