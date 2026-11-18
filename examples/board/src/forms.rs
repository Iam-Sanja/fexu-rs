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
