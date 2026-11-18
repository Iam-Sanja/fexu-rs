//! Built-in controls beyond the original text widgets, plus a checked custom hook.
//!
//! HTML attributes are hints. `bind_model` and `bind_declared` still decide whether a
//! value is accepted. A custom renderer receives the raw field only through
//! [`WidgetInput`]; request text is emitted with [`WidgetInput::escaped_name`] and
//! [`WidgetInput::escaped_value`]. Output that still contains an unescaped request
//! value is replaced before it reaches the page.

use crate::{escape, Field, Widget};

/// Values a custom widget may read. The strings are the request, not HTML.
#[derive(Debug, Clone, Copy)]
pub struct WidgetInput<'a> {
    pub name: &'a str,
    pub values: &'a [String],
    pub required: bool,
    pub class_name: &'a str,
}

impl WidgetInput<'_> {
    pub fn escaped_name(&self) -> String {
        escape(self.name)
    }

    pub fn escaped_value(&self) -> String {
        escape(self.values.first().map(String::as_str).unwrap_or(""))
    }

    pub fn escaped_values(&self) -> Vec<String> {
        self.values.iter().map(|value| escape(value)).collect()
    }

    pub fn escaped_class(&self) -> String {
        escape(self.class_name)
    }
}

/// Function a form author supplies for [`Widget::Custom`].
pub type WidgetRender = fn(WidgetInput<'_>) -> String;

struct Chrome<'a> {
    required: &'a str,
    described: &'a str,
    invalid: &'a str,
    class: &'a str,
}

pub(crate) fn render_special(
    field: &Field,
    input_name: &str,
    values: &[String],
    required: &str,
    described: &str,
    invalid: &str,
    class: &str,
) -> String {
    let chrome = Chrome {
        required,
        described,
        invalid,
        class,
    };
    let name = escape(input_name);
    let value = values.first().map(String::as_str).unwrap_or("");
    match field.widget {
        Widget::Date => temporal(&name, "date", value, &chrome),
        Widget::Time => temporal(&name, "time", value, &chrome),
        Widget::DateTime => temporal(&name, "datetime-local", value, &chrome),
        Widget::Decimal => format!(
            "<input id=\"{name}\" name=\"{name}\" type=\"text\" inputmode=\"decimal\" value=\"{value}\"{class}{required}{described}{invalid}>",
            value = escape(value),
            class = chrome.class,
            required = chrome.required,
            described = chrome.described,
            invalid = chrome.invalid,
        ),
        Widget::Hidden => format!(
            "<input id=\"{name}\" name=\"{name}\" type=\"hidden\" value=\"{value}\"{class}>",
            value = escape(value),
            class = chrome.class,
        ),
        Widget::Readonly => format!(
            "<span id=\"{name}\"{class}{described}{invalid}>{value}</span>",
            value = escape(value),
            class = chrome.class,
            described = chrome.described,
            invalid = chrome.invalid,
        ),
        Widget::Radio => choice_group(field, &name, values, "radio", true, &chrome),
        Widget::CheckboxGroup => choice_group(field, &name, values, "checkbox", false, &chrome),
        Widget::Custom(render) => {
            let input = WidgetInput {
                name: input_name,
                values,
                required: field.required,
                class_name: field.class_name,
            };
            let html = render(input);
            if request_leaks(input_name, values, field.class_name, &html) {
                format!(
                    "<input id=\"{name}\" name=\"{name}\" type=\"text\" value=\"{value}\"{class}>",
                    value = escape(value)
                )
            } else {
                html
            }
        }
        Widget::Text
        | Widget::Email
        | Widget::Password
        | Widget::Number
        | Widget::Url
        | Widget::Checkbox
        | Widget::Textarea
        | Widget::Select
        | Widget::SelectMultiple
        | Widget::File
        | Widget::Relation => String::new(),
    }
}

fn temporal(name: &str, kind: &str, value: &str, chrome: &Chrome<'_>) -> String {
    format!(
        "<input id=\"{name}\" name=\"{name}\" type=\"{kind}\" value=\"{value}\"{class}{required}{described}{invalid}>",
        value = escape(value),
        class = chrome.class,
        required = chrome.required,
        described = chrome.described,
        invalid = chrome.invalid,
    )
}

fn choice_group(
    field: &Field,
    name: &str,
    values: &[String],
    kind: &str,
    required_hint: bool,
    chrome: &Chrome<'_>,
) -> String {
    let mut html = format!(
        "<div id=\"{name}\"{class}{described}{invalid}>",
        class = chrome.class,
        described = chrome.described,
        invalid = chrome.invalid,
    );
    for (option, label) in field.options {
        let checked = if values.iter().any(|value| value == option) {
            " checked"
        } else {
            ""
        };
        let hint = if required_hint { chrome.required } else { "" };
        html.push_str(&format!(
            "<label><input type=\"{kind}\" name=\"{name}\" value=\"{option}\"{checked}{hint}>{label}</label>",
            option = escape(option),
            label = escape(label),
        ));
    }
    html.push_str("</div>");
    html
}

/// True when request text that needs escaping is still present after removing its escaped form.
fn request_leaks(name: &str, values: &[String], class_name: &str, html: &str) -> bool {
    leaks(name, html) || leaks(class_name, html) || values.iter().any(|value| leaks(value, html))
}

fn leaks(value: &str, html: &str) -> bool {
    if value.is_empty() || escape(value) == value {
        return false;
    }
    let stripped = html.replace(&escape(value), "");
    stripped.contains(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_output_cannot_keep_an_unescaped_request_value() {
        fn raw(input: WidgetInput<'_>) -> String {
            format!(
                "<b>{}</b>",
                input.values.first().map(String::as_str).unwrap_or("")
            )
        }
        fn safe(input: WidgetInput<'_>) -> String {
            format!("<b>{}</b>", input.escaped_value())
        }
        let field = Field::new("tone", "Tone", Widget::Custom(raw), false);
        let leaked = render_special(&field, "tone", &["<script>".to_owned()], "", "", "", "");
        assert!(leaked.contains("&lt;script&gt;"));
        assert!(!leaked.contains("<script>"));
        let field = Field::new("tone", "Tone", Widget::Custom(safe), false);
        let html = render_special(&field, "tone", &["<script>".to_owned()], "", "", "", "");
        assert!(html.starts_with("<b>&lt;script&gt;</b>"));
    }
}
