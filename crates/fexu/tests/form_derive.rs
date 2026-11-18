//! Model-aware form derive, including numeric limits, choices and decimals.

use fexu::core::{DatabaseKind, FexuModel};
use fexu::forms::{bind_formset, FexuForm, FormValues, SubmitMode};
use fexu::prelude::*;
use fexu::{FexuChoice, FexuForm};

#[derive(Clone, Copy, Debug, PartialEq, Eq, FexuChoice)]
enum ItemStatus {
    #[choice(value = "draft", de = "Entwurf", en = "Draft")]
    Draft,
    #[choice(value = "active", de = "Aktiv", en = "Active")]
    Active,
}

#[derive(Debug, fexu::FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(table = "derived_form_events")]
struct TimedEvent {
    #[field(primary_key, auto)]
    id: Uuid,
    starts: fexu::chrono::DateTime<fexu::chrono::Utc>,
}

#[derive(Debug, FexuForm)]
#[form(model = TimedEvent)]
struct TimedEventForm {
    starts: String,
}

#[derive(Debug, fexu::FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(table = "derived_form_items")]
struct DerivedItem {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required, min_length = 1, max_length = 40)]
    name: String,
    #[field(min = 0, max = 20)]
    stock: i64,
    #[field(precision = 8, scale = 2, min = 0)]
    price: Decimal,
    #[field(choice)]
    status: ItemStatus,
}

#[derive(Debug, PartialEq, Eq, FexuForm)]
#[form(model = DerivedItem)]
struct DerivedItemForm {
    name: String,
    stock: i64,
    price: String,
    status: String,
}

#[derive(Debug, FexuForm)]
#[form(model = DerivedItem)]
struct PatchItemForm {
    name: Option<String>,
    stock: Option<i64>,
    price: Option<String>,
    status: Option<String>,
}

#[test]
fn derive_uses_model_bounds_choices_and_patch() {
    assert!(DerivedItem::schema_sql(DatabaseKind::Sqlite).contains("derived_form_items"));
    let mut values = FormValues::default();
    values.insert("name", "Bolt");
    values.insert("stock", "21");
    values.insert("price", "1.50");
    values.insert("status", "active");
    let errors = DerivedItemForm::from_form(&values).unwrap_err();
    assert_eq!(errors.for_field("stock"), Some("validation.max_value"));
    values.insert("stock", "2");
    values.insert("status", "missing");
    let errors = DerivedItemForm::from_form(&values).unwrap_err();
    assert_eq!(errors.for_field("status"), Some("validation.choice"));
    values.insert("status", "active");
    values.insert("price", "-1.00");
    let errors = DerivedItemForm::from_form(&values).unwrap_err();
    assert_eq!(errors.for_field("price"), Some("validation.min_value"));
    values.insert("price", "1.50");
    values.insert("id", "nope");
    let errors = DerivedItemForm::from_form(&values).unwrap_err();
    assert_eq!(errors.for_field("id"), Some("validation.unknown_field"));
    values = FormValues::default();
    values.insert("name", "Bolt");
    values.insert("stock", "2");
    values.insert("price", "1.50");
    values.insert("status", "active");
    let form = DerivedItemForm::from_form(&values).unwrap();
    assert_eq!(form.stock, 2);
    assert_eq!(form.price, "1.50");
    let patched = PatchItemForm::from_submit(SubmitMode::Patch, &FormValues::default()).unwrap();
    assert_eq!(patched.name, None);
    assert_eq!(patched.stock, None);
    assert_eq!(patched.price, None);
    assert_eq!(patched.status, None);
    let mut rows = FormValues::default();
    rows.insert("items_TOTAL_FORMS", "1");
    rows.insert("items_0_name", "Bolt");
    rows.insert("items_0_stock", "2");
    rows.insert("items_0_price", "1.50");
    rows.insert("items_0_status", "draft");
    let parsed = bind_formset(&rows, "items", 3, SubmitMode::Create, |mode, row| {
        DerivedItemForm::from_submit(mode, row)
    })
    .unwrap();
    assert_eq!(parsed[0].status, "draft");
}

fn reject_admin(value: &str) -> Result<(), &'static str> {
    if value.eq_ignore_ascii_case("admin") {
        Err("validation.choice")
    } else {
        Ok(())
    }
}

fn signup_names_differ(form: &SignupForm) -> Result<(), fexu::forms::FormErrors> {
    if form
        .email
        .split('@')
        .next()
        .is_some_and(|local| local.eq_ignore_ascii_case(&form.name))
    {
        let mut errors = fexu::forms::FormErrors::default();
        errors.push_form("validation.mismatch");
        return Err(errors);
    }
    Ok(())
}

#[derive(Debug, FexuForm)]
#[form(cross = signup_names_differ)]
struct SignupForm {
    #[form(
        label = "Name",
        min_length = 3,
        max_length = 40,
        check = reject_admin,
        placeholder = "Ada"
    )]
    name: String,
    #[form(widget = "email", label = "Email")]
    email: String,
    #[form(widget = "select_multiple", choices = "red,blue")]
    tags: Vec<String>,
}

#[derive(Debug, FexuForm)]
#[form(model = DerivedItem)]
struct StaffPriceForm {
    name: String,
    #[form(readonly_unless = "staff")]
    price: Option<String>,
    #[form(required_when = "staff")]
    status: Option<String>,
}

#[test]
fn standalone_form_and_context_rules_compile_and_reject_forged_fields() {
    let widgets = SignupForm::widgets();
    assert_eq!(widgets[0].label, "Name");
    assert_eq!(widgets[0].placeholder, "Ada");
    assert_eq!(widgets[1].widget, fexu::forms::Widget::Email);
    assert_eq!(widgets[2].widget, fexu::forms::Widget::SelectMultiple);
    let posted = FormValues::parse_urlencoded_multi(
        "name=Ada&email=ada@example.com&tags=red&tags=blue&role=admin",
        &["tags"],
    )
    .unwrap();
    assert_eq!(
        SignupForm::from_form(&posted)
            .unwrap_err()
            .for_field("role"),
        Some("validation.unknown_field")
    );
    let posted =
        FormValues::parse_urlencoded_multi("name=Ann&email=ada@example.com&tags=red", &["tags"])
            .unwrap();
    let form = SignupForm::from_form(&posted).unwrap();
    assert_eq!(form.tags, ["red"]);
    let posted = FormValues::parse_urlencoded("name=Ada&email=Ada@example.com").unwrap();
    assert_eq!(
        SignupForm::from_form(&posted).unwrap_err().form_error(),
        Some("validation.mismatch")
    );
    let mut price = FormValues::default();
    price.insert("name", "Bolt");
    price.insert("price", "1.50");
    let denied = StaffPriceForm::from_submit_in(
        SubmitMode::Patch,
        &price,
        &fexu::forms::FormContext {
            mode: SubmitMode::Patch,
            flags: &[],
        },
    )
    .unwrap_err();
    assert_eq!(denied.for_field("price"), Some("validation.readonly"));
    price.insert("stock", "1");
    assert_eq!(
        StaffPriceForm::from_submit(SubmitMode::Create, &price)
            .unwrap_err()
            .for_field("stock"),
        Some("validation.unknown_field")
    );
    let mut staff = FormValues::default();
    staff.insert("name", "Bolt");
    staff.insert("price", "1.50");
    let missing_status = StaffPriceForm::from_submit_in(
        SubmitMode::Create,
        &staff,
        &fexu::forms::FormContext {
            mode: SubmitMode::Create,
            flags: &["staff"],
        },
    )
    .unwrap_err();
    assert_eq!(
        missing_status.for_field("status"),
        Some("validation.required")
    );
    staff.insert("status", "active");
    let allowed = StaffPriceForm::from_submit_in(
        SubmitMode::Create,
        &staff,
        &fexu::forms::FormContext {
            mode: SubmitMode::Create,
            flags: &["staff"],
        },
    )
    .unwrap();
    assert_eq!(allowed.name, "Bolt");
    assert_eq!(allowed.price.as_deref(), Some("1.50"));
    let mut civil = FormValues::default();
    civil.insert("starts", "2027-02-08T09:00");
    assert_eq!(
        TimedEventForm::from_submit(SubmitMode::Create, &civil)
            .unwrap_err()
            .for_field("starts"),
        Some("validation.datetime")
    );
    let berlin = fexu::forms::Zone::parse("Europe/Berlin").unwrap();
    let zoned = TimedEventForm::from_submit_in_zone(
        SubmitMode::Create,
        &civil,
        &fexu::forms::FormContext::create(),
        Some(berlin),
    )
    .unwrap();
    assert_eq!(zoned.starts, "2027-02-08T08:00:00Z");
    let utc = TimedEventForm::from_submit_in_zone(
        SubmitMode::Create,
        &civil,
        &fexu::forms::FormContext::create(),
        Some(fexu::forms::Zone::utc()),
    )
    .unwrap();
    assert_eq!(utc.starts, "2027-02-08T09:00:00Z");
    assert_eq!(allowed.status.as_deref(), Some("active"));
}

#[derive(Debug, fexu::FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(table = "derived_form_labels")]
struct LimitedLabel {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required, min_length = 2, max_length = 3)]
    code: String,
    #[field(format = "email")]
    email: String,
    #[field(format = "slug", max_length = 80)]
    site: Option<String>,
}

#[derive(Debug, FexuForm)]
#[form(model = LimitedLabel)]
struct LimitedLabelForm {
    code: String,
    email: String,
    site: Option<String>,
}

#[test]
fn derive_applies_model_contracts_for_html_and_json() {
    let widgets = LimitedLabelForm::widgets().unwrap();
    assert_eq!(widgets[0].widget, fexu::forms::Widget::Text);
    assert_eq!(widgets[1].widget, fexu::forms::Widget::Email);
    assert_eq!(widgets[2].widget, fexu::forms::Widget::Text);
    let mut html = FormValues::default();
    html.insert("code", "x");
    html.insert("email", "ada@example.com");
    assert_eq!(
        LimitedLabelForm::from_form(&html)
            .unwrap_err()
            .for_field("code"),
        Some("validation.min_length")
    );
    let json = FormValues::parse_json(r#"{"code":"too long","email":"ada@example.com"}"#).unwrap();
    assert_eq!(
        LimitedLabelForm::from_submit(SubmitMode::Create, &json)
            .unwrap_err()
            .for_field("code"),
        Some("validation.max_length")
    );
    let mut bad_mail = FormValues::default();
    bad_mail.insert("code", "ab");
    bad_mail.insert("email", "not-an-email");
    assert_eq!(
        LimitedLabelForm::from_form(&bad_mail)
            .unwrap_err()
            .for_field("email"),
        Some("validation.email")
    );
    let mut bad_url = FormValues::default();
    bad_url.insert("code", "ab");
    bad_url.insert("email", "ada@example.com");
    bad_url.insert("site", "Not a slug");
    assert_eq!(
        LimitedLabelForm::from_form(&bad_url)
            .unwrap_err()
            .for_field("site"),
        Some("validation.slug")
    );
    let mut empty_site = FormValues::default();
    empty_site.insert("code", "ab");
    empty_site.insert("email", "ada@example.com");
    empty_site.insert("site", "   ");
    let form = LimitedLabelForm::from_form(&empty_site).unwrap();
    assert_eq!(form.code, "ab");
    assert_eq!(form.email, "ada@example.com");
    assert_eq!(form.site, None);
    let mut missing = FormValues::default();
    missing.insert("email", "ada@example.com");
    assert_eq!(
        LimitedLabelForm::from_form(&missing)
            .unwrap_err()
            .for_field("code"),
        Some("validation.required")
    );
    let mut posted =
        FormValues::parse_urlencoded("code=ab&email=ada@example.com&id=forged").unwrap();
    assert_eq!(
        LimitedLabelForm::from_form(&posted)
            .unwrap_err()
            .for_field("id"),
        Some("validation.unknown_field")
    );
    posted.remove("id");
    let saved = LimitedLabelForm::from_form(&posted).unwrap();
    assert_eq!(saved.code, "ab");
    assert_eq!(saved.email, "ada@example.com");
}
