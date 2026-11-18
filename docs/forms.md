# Forms

**Current package version:** `0.1.0` (experimental). Widget and model-binding additions are on `main`; historical `v0.19` is an internal milestone, not a registry version. `required_date`, `optional_date`, `required_time`, `optional_time`, `required_datetime_utc` and `required_datetime_local` check the canonical strings. `required_datetime_local` still reads a zone-free value as UTC. `required_datetime_in_zone` and `render_datetime_local` take a `ProjectTimeZone`. `ProjectTimeZone::utc()` preserves that contract. `ProjectTimeZone::parse("Europe/Berlin")` converts civil time, rejects DST gaps, and requires `earlier` or `later` for an overlap. `bind_model` still expects the canonical UTC text from `canonical_utc` for a `DateTime` column. See [date-time-types.md](date-time-types.md).

## Responsibility

`fexu-forms` checks URL-encoded input and provides a typed Rust object or field error. Forms are not bound to a database table. Contact, search and filter can thus be described without creating a `FexuModel`.

## Crates and files

- `crates/fexu-forms/src/lib.rs`
- Shared validation rules: `crates/fexu-core/src/validation.rs`
- Example: `examples/myshop/src/contact.rs`
- Generator: `crates/fexu-cli/src/scaffold.rs`, function `contact_handlers`

The facade exports the module as `fexu::forms`.

## Public API

`FexuForm::from_form` accepts `FormValues`; `ContactForm` is the supplied example. Helpers include `required_text`, `optional_text`, `required_i64`, `optional_i64`, `required_bool`, `optional_bool`, `required_uuid`, `optional_uuid`, `required_choice`, `optional_choice` and `required_postal`. `render_form` still accepts `InputSpec`. `render_fields` renders `Field` widgets: text, email, password, number, URL, checkbox, textarea and select. Labels, help, placeholders, the `required` attribute and field errors are escaped. `aria-describedby` points at `{name}-error` and `{name}-help` when those texts exist. A selected option is marked only when its value equals the current field value. Password values are not written back into the page. `message` maps stable codes to German and English text. `FormValues::parse_urlencoded` limits the body to 65536 bytes and rejects unknown fields through `reject_unknown`. `csrf_token` is allowed as a control field rather than form data.

`model_fields` builds fields from `ColumnSpec` and `AdminFieldMeta`, but only for names in the explicit allow-list. A name that is not a column, a duplicate, a primary key, or an admin field marked sensitive, readonly or hidden returns `FormSetupError` and creates no field. `bind_model` distinguishes `SubmitMode::Create`, `Replace` and `Patch`. Create and Replace require every required field and turn a missing optional field into `BoundValue::Null`. Patch keeps a missing key as `BoundValue::Unchanged`. Posted names outside the field list produce `validation.unknown_field`, including when a client adds a column the allow-list omitted. `choices` rejects values that are not in the supplied list with `validation.choice`.

`FormValues::parse_urlencoded` and `FormValues::parse_json` both feed `bind_submission`. Duplicate keys are `validation.duplicate`. Names in the protected list are `validation.readonly` and are removed before binding, so a forged readonly or disabled field cannot overwrite the stored value. `apply_contracts` copies `min_length`, `max_length` and `format` from `FieldContract`. Lengths count Unicode scalar values. `render_relation_select` renders only the options the caller already limited with `QueryScope`. `inspect_upload` rejects traversal, size, a content type outside the policy, and image bytes that do not match PNG, JPEG, GIF or WebP. `plan_formset` counts creates, updates and deletes. The caller writes those rows in one database transaction and rolls the transaction back when validation fails.

`#[derive(FexuForm)]` with `#[form(model = Type)]` builds a form from the struct's columns. The struct is the allow-list: columns that are absent stay out, including the primary key. A second struct can list a different subset of the same model. `from_submit`, `from_submit_in` and `from_submit_in_zone` call `apply_bounds` and `apply_contracts` before binding, so `min_length`, `max_length`, `format` (`slug`, `email` or `url`), numeric limits and choices are the same for HTML and JSON. `bind_posted_contracts` runs that same check on the text, number, decimal, boolean, uuid and choice values an admin HTML post actually sent. Date, time, datetime and JSON stay on their existing parsers. An omitted key, including an unchecked checkbox, is left to the repository. An empty contract does nothing. `from_submit` uses an empty flag set and still requires canonical UTC for a `DateTime` column. `from_submit_in` takes a `FormContext`. `from_submit_in_zone` converts civil time only for the `ProjectTimeZone` the caller passes, then stores `canonical_utc`. `ProjectTimeZone::utc()` does not shift the clock. `#[form(readonly_unless = "staff")]` rejects a posted value unless that flag is present. `#[form(required_when = "staff")]` makes the field required only when the flag is present. `widgets()` returns labels, help, placeholders and the widget selected from the column kind or `format`. A `DateTime` column stays a text widget so the UTC contract is unchanged; `widget = "datetime"` is the opt-in civil-time control.

A derive without `model` is a standalone form. Field types are `String`, `i64`, `bool`, `Vec<String>` and `Option` of `String`, `i64` or `bool`. Attributes set `label`, `help`, `placeholder`, `widget`, `widget_fn`, `class`, `min_length`, `max_length`, `choices`, `check`, `required`, `readonly`, `disabled` and the same flag rules. `disabled` and `readonly` both reject a posted value with `validation.readonly`. Their HTML is a span without `name`; `disabled` also sets the `disabled` attribute. That attribute is not the security check. A model field still uses `readonly_unless` rather than `disabled` or `readonly`. `#[form(check = function)]` runs after the built-in text rules and returns one stable code. `#[form(cross = function)]` runs only after every field is valid and may call `FormErrors::push_form`. Field codes stay in declaration order; `validation_order` lists the form code first. `Vec<String>` is the only repeated value. `parse_urlencoded` and `parse_json` still reject every duplicate key. `parse_urlencoded_multi` and `parse_json_multi` repeat only the names passed in, and still reject duplicates of every other name.

`render_configured` accepts a `FormRender`. `render_form` and `render_fields` keep their previous signatures and use that renderer with an empty prefix. Field order is the slice order, and `order_fields` requires a permutation of those names. `prefix` changes the HTML name and id to `{prefix}_{field}` and leaves `csrf_token` alone. `unprefix` restores the declared names before `from_form` or `from_submit`. A prefixed name and the same bare name together are `validation.duplicate`. `class_name` and `css_class` add a class attribute. `FieldOverride` replaces one field message. `FormTemplate` and `FieldTemplate` receive fragments that are already escaped; they do not receive the raw request. The default template keeps labels, `aria-describedby`, inline errors and a summary list. Password values are still not written back. A readonly widget renders text without a `name`, so it is not submitted. A hidden widget omits the visible label. Checkbox groups do not set the HTML `required` attribute on every box. Date, time and datetime-local inputs are hints; the server parsers stay in charge. `Widget::Custom` must escape request text through `WidgetInput`. Markup that still contains that text is replaced with an escaped input.

`constraint_errors` and `constraint_errors_from_message` turn a unique or foreign-key failure into `validation.unique` or `validation.relation_denied`. The only recognised messages are `unique constraint violated` and `foreign key constraint violated`. Any other database text is left untouched. Column names come from the caller, not from the database message. An empty column list is a form-level code.

`bind_create` and `bind_update` are the create and patch contracts around `bind_submission`. `bind_in_context` applies `FieldAccess` on an allow-list the caller already built with `model_fields`. `Presentation::unbound` shows initial values. `Presentation::bound` shows the posted values and does not fill gaps from the initial values. `posted_csrf` reads the control field. The form crate does not authenticate that token; the handler still uses `AuthService`.

`bind_formset` accepts at most 100 rows named `{prefix}_{index}_{field}`. `bind_model_formset` uses the same names and adds `MIN_FORMS`, `MAX_FORMS`, `{index}_id`, `{index}_DELETE` and `{index}_ORDER`. A posted minimum or maximum that does not match the configuration is `validation.formset_size`. Duplicate management keys stay `validation.duplicate` at parse time. A repeated order is `validation.formset_order`. The posted `FormValues` are left unchanged when a row fails, so the handler can render them again. `bind_inline_formset` takes the parent id from the caller. A different posted parent id, or a child id outside `owned_ids`, is `validation.relation_denied`. Editing the parent does not add those ids.

`bind_relation` accepts a foreign key, a one-to-one id, or a many-to-many list. The ids in `allowed` are the only ones that can be saved. They must already have been loaded with the caller's `QueryScope`. An id outside `known` is `validation.relation_missing`. An id in `known` but not in `allowed` is `validation.relation_denied`. Pass the same list for both when that difference must stay hidden. `render_relation_widget` renders at most 100 options. A larger slice is `form.relation_page`. `relation_window` returns one page of that size. Search and picker widgets add a query field and do not emit the rest of the catalog. Readonly widgets echo only selected options that are also in the page.

`store_upload` rejects `..`, slashes and NUL, then stores the bytes through `LocalStorage`. Numeric `min` and `max` come from the model column metadata. The HTML `required` attribute does not replace the server check. The form crate does not open a database transaction. `examples/board` saves a project and its tasks with `save_project_with_tasks`, which uses `conclude` so a rejected child rolls the project back.

## Data flow

The handler reads the body, builds `FormValues` and calls `from_form`. Valid values are typed. Errors remain codes per field. `render_form` escaped names, values and messages. The success page of the contact example does not save the message and does not send an email.

## Configuration

Forms do not have their own `fexu.toml` section. The maximum URL-encoded size is in `FormValues::parse_urlencoded`. Larger requests are previously rejected by the body limit middleware if `fexu::web::prepare` or `fexu::middleware::apply` is used.

## Security

HTML output is escaped. Unknown fields return `validation.unknown_field` instead of being silently accepted. Validation runs on the server. A cookie-authenticated form submits the same HMAC token used by `x-csrf-token` in its `csrf_token` field. Obtain a user token with `AuthService::csrf_from_request`. A public form uses `ensure_anonymous_session` and `authenticate_browser_form`. myshop `/contact` does that when an `AuthService` is configured. Without an auth service every contact POST is rejected and no session is created. Generated public JSON routes are covered by the app-router cookie check: a session cookie still needs CSRF, and a cookie-less request stays public.

## Error cases

Codes include `validation.required`, `validation.min_length`, `validation.max_length`, `validation.min_value`, `validation.max_value`, `validation.email`, `validation.url`, `validation.invalid_type`, `validation.uuid`, `validation.unknown_field`, `validation.choice`, `validation.postal`, `validation.mismatch`, `validation.readonly`, `validation.duplicate`, `validation.unique`, `validation.relation`, `validation.relation_missing`, `validation.relation_denied`, `validation.formset_size`, `validation.formset_order`, `form.relation_page` and `form.csrf`. Field codes use the field name. `push_form` stores a form code under the empty name, and `form_error` reads it back. `ContactForm` returns `validation.mismatch` when the name equals the local part of the email address. `topic` is optional and must be `order`, `product` or `other`. For unknown codes, `message` returns `missing message`. Setup codes are `form.unknown_column`, `form.duplicate_column` and `form.protected_column`.

`required_column_choice`, `choice_options` and `render_choice_select` build a select from `ColumnSpec::choices`. `required_choice` still checks a caller-supplied option list. `required_decimal` rejects values that do not fit the declared precision and scale. Labels follow `de` or any other locale as English. Unknown choices use `validation.choice` and bad decimals use `validation.decimal`. See [models.md](models.md).

## Example

```rust
let values = fexu::forms::FormValues::parse_urlencoded(&body).expect("form body");
let form = fexu::forms::ContactForm::from_form(&values).expect("contact");
let admin = Product::admin_meta().map(|meta| meta.fields).unwrap_or(&[]);
let fields = fexu::forms::model_fields(Product::columns(), admin, &["title"]).expect("fields");
let bound = fexu::forms::bind_model(
    fexu::forms::SubmitMode::Replace,
    &fields,
    &values,
    &[],
)
.expect("bound");
let _checked = &form.name;
let _title = &bound[0].value;
```

## Tests

`cargo test -p fexu-forms` and `cargo test -p fexu --test form_derive`. Covered are mandatory fields, length, email, URL, number, UUID, optional fields, cross validation, unknown fields, escaping, select choices, postal codes, prefilled values, password suppression, the model allow-list, repeated keys, context flags, the standalone derive, relation ids, relation pages of 0, 1, 100 and a rejected 1000-option render, and formset management fields. `cargo test -p board --test forms` writes one note through the publish form and revises it through the title form on SQLite. `cargo test -p board --test relations` saves a project with tasks, rejects a foreign lead and a foreign parent id, and rolls back a child write. The PostgreSQL copies are ignored unless `DATABASE_URL` points at an approved disposable database. `cargo test -p fexu-admin --lib relations::` renders those lookup pages through `render_scoped_page`.

## Limitations

File and image fields use `MediaField` with `read_media_parts`, `bind_media`, `render_media_field` and `finish_media`. The multipart parse and `bind_media` write nothing. `finish_media` stages, then polls the database future, then commits or aborts storage. A database rollback does not by itself delete or retain files. `CommitReport` can still list a failed retain or a failed removal after the database has committed. `store_upload` still stores one file immediately. Admin record images remain database blobs and are described in [uploads.md](uploads.md). A formset plan does not open a transaction by itself. Remote storage is not included. `parse_json` accepts only a flat object of strings, numbers, booleans and null. `parse_json_multi` additionally accepts an array of those scalars for declared repeated names.

## Example app

`examples/board/tests/acceptance.rs` posts the live `/board/items/new` form and the JSON route `/board_notes`. A title of 81 Unicode scalar values is `validation.max_length` on both, the HTML page shows the localized sentence and the posted title, and the table stays empty. A repeated title, an unknown name and a forged `owner_id` are rejected on that same POST, and the localized sentence is in the form. `examples/board/tests/relations.rs` rolls a foreign child back inside one database transaction on SQLite and PostgreSQL. `crates/fexu-storage/src/http.rs` posts an oversized body and a mismatched multipart type to `/uploads`, and another owner cannot read the file. `tests/browser/login-noscript.spec.js` submits the login form with JavaScript disabled. `examples/board/src/forms.rs` declares `NotePublishForm` and `NoteReviseForm` for the same `Note` model. Publish accepts `title` and `body`. Revise accepts `title`. `owner_id` is on neither form. The repository write in `examples/board/tests/forms.rs` still passes `QueryScope::Equals` for `owner_id`. `NoteSearchForm` has no model. `NoteFilterForm` is a second standalone form: `tone_widget` escapes its value, `tone_is_word` rejects other text, and `note_form_template` wraps the escaped fragments. `NoteStampForm` rejects a forged disabled owner and a forged readonly channel. `note_cover_field` and `note_cover_html` use `MediaField` for one required file. `ProjectForm` and `TaskLineForm` are the relation and inline formset pair saved by `save_project_with_tasks`.

## Not in this contract

`bind_formset` and `plan_formset` remain for callers that only need a row count. The caller opens the database transaction and rolls it back. `inspect_upload` and `store_upload` remain the immediate helpers. `MediaField` is the form-facing replace, clear and readonly API and is described in [storage.md](storage.md). Wizards are not implemented. The next wizard step would be a list of the existing form types plus the index of the current step, with each step calling `from_form_in` or `from_submit_in` and no step writing until the last one succeeds. `render_relation_select` remains for a caller that already holds a small option list. `bind_relation` does not widen `QueryScope`. `from_submit` calls `apply_contracts`. The form crate still does not authenticate `csrf_token`. The admin HTTP picker and `save_with_children` remain the admin paths; `render_scoped_page` renders one `lookup_page` through the same widget.
