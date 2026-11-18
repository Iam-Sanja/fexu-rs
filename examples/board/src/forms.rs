//! Two model forms for `Note`, plus one standalone search form.
//!
//! Neither model form lists `id` or `owner_id`. The caller still applies
//! `QueryScope` and the object policy when it writes.

use fexu::forms::FormErrors;

use crate::{Note, Project, Task};

#[derive(Debug, fexu::FexuForm)]
#[form(model = Note)]
pub struct NotePublishForm {
    pub title: String,
    pub body: Option<String>,
}

#[derive(Debug, fexu::FexuForm)]
#[form(model = Note)]
pub struct NoteReviseForm {
    pub title: String,
}

pub fn tone_widget(input: fexu::forms::WidgetInput<'_>) -> String {
    format!(
        "<input id=\"{name}\" name=\"{name}\" type=\"text\" class=\"{class}\" data-fexu-widget=\"tone\" value=\"{value}\">",
        name = input.escaped_name(),
        class = input.escaped_class(),
        value = input.escaped_value(),
    )
}

pub fn note_form_template(parts: fexu::forms::FormFragments<'_>) -> String {
    format!(
        "<section class=\"note-form\">{start}{summary}{controls}{submit}{end}</section>",
        start = parts.start,
        summary = parts.summary,
        controls = parts.controls,
        submit = parts.submit,
        end = parts.end,
    )
}

fn tone_is_word(value: &str) -> Result<(), &'static str> {
    if !value.is_empty() && value.chars().all(|char| char.is_ascii_alphabetic()) {
        Ok(())
    } else {
        Err("validation.invalid_type")
    }
}

fn filter_rejects_none(form: &NoteFilterForm) -> Result<(), FormErrors> {
    if form.tone.eq_ignore_ascii_case("none") {
        let mut errors = FormErrors::default();
        errors.push_form("validation.mismatch");
        return Err(errors);
    }
    Ok(())
}

#[derive(Debug, fexu::FexuForm)]
#[form(cross = filter_rejects_none)]
pub struct NoteFilterForm {
    #[form(
        label = "Tone",
        widget_fn = tone_widget,
        check = tone_is_word,
        class = "tone",
        placeholder = "calm"
    )]
    pub tone: String,
}

fn search_needs_a_term(form: &NoteSearchForm) -> Result<(), FormErrors> {
    if form.title.is_none() && form.tags.is_empty() {
        let mut errors = FormErrors::default();
        errors.push_form("validation.required");
        return Err(errors);
    }
    Ok(())
}

#[derive(Debug, fexu::FexuForm)]
#[form(cross = search_needs_a_term)]
pub struct NoteSearchForm {
    #[form(label = "Title", placeholder = "notes", max_length = 80)]
    pub title: Option<String>,
    #[form(widget = "select_multiple", choices = "open,done", label = "Tags")]
    pub tags: Vec<String>,
}

#[derive(Debug, fexu::FexuForm)]
#[form(model = Project)]
pub struct ProjectForm {
    #[form(label = "Project")]
    pub title: String,
}

#[derive(Debug, fexu::FexuForm)]
#[form(model = Task)]
pub struct TaskLineForm {
    #[form(label = "Task")]
    pub title: String,
}

/// Standalone form. `owner` and `channel` are not successful controls.
#[derive(Debug, fexu::FexuForm)]
pub struct NoteStampForm {
    #[form(label = "Title", min_length = 1, max_length = 80)]
    pub title: String,
    #[form(disabled, label = "Owner")]
    pub owner: Option<String>,
    #[form(readonly, label = "Channel")]
    pub channel: Option<String>,
}

/// Required file field. Bytes stay in the request until `bind_media` accepts them.
pub fn note_cover_field<'a>(upload: Option<(&'a str, &'a [u8])>) -> fexu::forms::MediaField<'a> {
    let field = fexu::forms::MediaField::file("cover").required();
    match upload {
        Some((name, bytes)) => field.upload(name, Some("text/plain"), bytes),
        None => field,
    }
}

pub fn note_cover_html(error: Option<&str>, current: Option<&str>) -> String {
    fexu::forms::render_media_field(fexu::forms::MediaView {
        name: "cover",
        label: "Cover",
        image: false,
        required: true,
        readonly: false,
        current_name: current,
        error,
    })
}
