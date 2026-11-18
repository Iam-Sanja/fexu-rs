use fexu::forms::{FexuForm, FormRender, FormValues, Widget};

use board::{note_form_template, NoteFilterForm, NotePublishForm, NoteReviseForm};

#[test]
fn filter_form_escapes_a_custom_widget_and_keeps_two_model_forms() {
    let publish = NotePublishForm::widgets().expect("publish widgets");
    let revise = NoteReviseForm::widgets().expect("revise widgets");
    assert_eq!(
        publish.iter().map(|field| field.name).collect::<Vec<_>>(),
        ["title", "body"]
    );
    assert_eq!(
        revise.iter().map(|field| field.name).collect::<Vec<_>>(),
        ["title"]
    );
    let mut forged = FormValues::default();
    forged.insert("title", "Visible");
    forged.insert("owner_id", "forged");
    assert_eq!(
        NotePublishForm::from_form(&forged)
            .unwrap_err()
            .for_field("owner_id"),
        Some("validation.unknown_field")
    );
    forged.remove("owner_id");
    forged.insert("title", "x".repeat(81));
    assert_eq!(
        NotePublishForm::from_form(&forged)
            .unwrap_err()
            .for_field("title"),
        Some("validation.max_length")
    );
    let json = FormValues::parse_json(&format!("{{\"title\":\"{}\"}}", "x".repeat(81))).unwrap();
    assert_eq!(
        NotePublishForm::from_form(&json)
            .unwrap_err()
            .for_field("title"),
        Some("validation.max_length")
    );

    let widgets = NoteFilterForm::widgets();
    assert!(matches!(widgets[0].widget, Widget::Custom(_)));
    assert_eq!(widgets[0].class_name, "tone");
    let mut posted = FormValues::default();
    posted.insert("tone", "<script>");
    assert_eq!(
        NoteFilterForm::from_form(&posted)
            .unwrap_err()
            .for_field("tone"),
        Some("validation.invalid_type")
    );
    posted.insert("tone", "none");
    assert_eq!(
        NoteFilterForm::from_form(&posted).unwrap_err().form_error(),
        Some("validation.mismatch")
    );
    posted.insert("tone", "calm");
    assert_eq!(NoteFilterForm::from_form(&posted).unwrap().tone, "calm");

    let mut errors = fexu::forms::FormErrors::default();
    errors.push("tone", "validation.invalid_type");
    let mut render = FormRender::new("/board/filter", &posted, &errors, &widgets, "en");
    render.form_template = Some(note_form_template);
    render.class_name = "filter";
    let html = fexu::forms::render_configured(render);
    assert!(html.contains("class=\"note-form\""));
    assert!(html.contains("data-fexu-widget=\"tone\""));
    assert!(html.contains("value=\"calm\""));
    posted.insert("tone", "<script>");
    let mut render = FormRender::new("/board/filter", &posted, &errors, &widgets, "en");
    render.form_template = Some(note_form_template);
    let html = fexu::forms::render_configured(render);
    assert!(html.contains("&lt;script&gt;"));
    assert!(!html.contains("<script>"));
}
