# Fexu internationalization architecture (RFC)

> **Status: Proposed RFC.** Tracking issue: [#148](https://github.com/Iam-Sanja/fexu-rs/issues/148).
> Step 1 is partial: the admin and the CLI ship embedded `en` and `de` catalogs. The packaging, pluralization and translator-import rules below are not implemented.
> Step 2 is partial: developer diagnostics and the CLI default are English-first. German remains for locale `de` and for `FEXU_LOCALE=de`. German comments and the internal migration states `angewendet` and `ausstehend` remain.
> Steps 3–6 are open. `#[field(translatable)]`, Settings → Translations and record-translation storage are not a public API.

## Goals

1. Fexu core, Rust comments and rustdoc, developer diagnostics, default generated applications, documentation, and built-in admin source text are **English-first**.
2. Every *officially released* locale catalog ships with the corresponding Fexu crates. Newly created projects can enable any shipped language **offline**; they do not download or install language packages.
3. Translators may maintain each language in an independent project/repository. CI imports approved snapshots at release time; translators do not have to modify Rust source.
4. Applications define translatable **labels** (apps, models, fields, actions, choices) and **record content** through small, discoverable model metadata extensions.
5. The admin has one project-wide translations area for labels and one field-level record translations editor. Both enforce existing security rules and avoid N+1 queries.
6. Existing single-language projects, API responses, migrations, permissions and database records remain compatible unless an explicitly documented migration is required.

## Non-goals

- Runtime translation by a third-party service, automatic machine translation or network fetching.
- Renaming technical identifiers (`products`, `price_cents`, API routes, permission codes, database tables/columns) when the UI locale changes.
- Translating arbitrary Rust source identifiers or user-provided data automatically.
- Claiming that displaying translated values automatically provides translated search/sort semantics.
- Requiring a separate Rust crate for each language.

## Definitions and source of truth

| Category | Canonical source | Runtime storage | Editable by project admin? |
| --- | --- | --- | --- |
| Built-in UI message | English core catalog / stable message ID | Bundled release artifact | No; project override may be designed separately |
| Framework documentation | English Markdown / rustdoc | Git repository | No |
| App/model/field label | App-provided English source label and stable namespaced key | Registered metadata + project overrides | Yes, within enabled locales |
| Record source value | Ordinary model column plus an explicit source-locale contract | Existing model row | Yes, through ordinary record permission |
| Record translation | Model/record/field/locale/value | Translation storage | Yes, with permission to edit source record |

English is the **framework** fallback. It is not automatically the source language of existing application content: a source value such as `Kerze` may have been authored in German.

## Catalog packaging

Use gettext-compatible PO for human translation and a validated, indexed representation (for example compiled MO or generated Rust-embedded catalog data) for efficient offline runtime lookup.

- Core maintains stable, namespaced IDs such as `admin.action.save`. English text is the canonical source; `de`, `fr`, etc. supply translations. Avoid making the English display sentence itself the long-lived ID.
- Support plural forms, typed parameters/placeholders and contextual meanings from the beginning. CI rejects incompatible placeholders and invalid catalog syntax.
- Translation projects export a versioned, reviewed catalog snapshot; the Fexu release process copies/pins the approved snapshot. No network dependency at runtime.
- Provide locale normalization and fallback in one service; never concatenate user-provided locale strings into filesystem paths.
- A locale is **shipped** if bundled in the crate; it is **enabled** if the project has selected it. The account switcher offers only enabled locales. Admin-generated missing-translation reports must not expose sensitive records.
- Preserve `en` and `de` compatibility during migration from `crates/fexu-admin/src/localization.rs`. Document how old keys map to new IDs.
- Translators work only with catalog files and contributor instructions. A new language is added to a future official release after catalog QA; no additional end-user download is needed.

Illustrative PO entry (exact catalog syntax/loader subject to implementation review):

```po
msgctxt "admin.action.save"
msgid "Save"
msgstr "Speichern"
```

## Project locale configuration

A project chooses `default_locale`, optional `content_source_locale` and `supported_locales` (enabled subset of shipped locales). Locale selection precedence should be explicit and tested for account preference/cookie, request language, project default and English fallback.

- A project enabling `en`, `de` and `fr` shows those languages in the account switcher and translation screens, only if those catalogs actually ship.
- Missing translated **UI** keys fall back to English without showing raw internal keys.
- Missing translated **record** fields fall back to the record's source value and are visually marked as missing in editing screens.
- Locale values are BCP-47-like identifiers with a documented normalization policy; begin with `en` and `de` and make future regional variants possible.
- Locale formatting for numbers, dates and money needs its own explicit contract and does not reuse raw label translation.

## App/model/field labels

Register stable namespaced keys when an app/model is registered. Use application identity and model/field identifiers, not translated text, for key identity. Illustrative IDs:

```text
products.model.singular
products.model.plural
products.field.name
products.field.price_cents
products.action.publish
```

Desired *illustrative* Rust declaration (**not an existing public API**):

```rust
#[derive(FexuModel)]
#[fexu(
    table = "products",
    admin = true,
    admin_label = "Product",
    admin_label_plural = "Products"
)]
pub struct Product {
    #[field(required, min_length = 3, max_length = 150, translatable)]
    pub name: String,

    pub price_cents: i64,
}
```

Possible addition: `#[fexu(translatable = true)]` to explicitly expose singular/plural model labels. **Design decision outstanding:** whether labels are always registered (recommended default) or require the model-level marker. Field-content translation remains strictly opt-in.

**Settings → Translations** must list label keys, English source, language columns, completion status, filtering by app/model and permission-restricted editing. Values should be exportable/importable (PO or equivalent) for reproducibility. A registered label is not a database record translation.

**Versioning question:** display-label changes, key renames and removed fields must not silently remap existing overrides. Specify alias/migration and collision rules before implementation.

## Translated record fields

Only explicitly marked supported text fields are translatable; not identifiers, numeric values, hashes, secrets, technical keys or arbitrary JSON by default.

An initial logical translation key consists of `(model_key, record_id, field_name, locale)`; enforce uniqueness. Decide the actual physical representation only after checking SQLite/PostgreSQL schema rules and existing model registration. Translating source content must **not** add `name_de` / `name_fr` columns to every model.

### Source locale

Each project/model/record must have an explicit way to determine the source locale, with deterministic behavior for existing rows. Never infer English from Fexu's own default or infer language from a string's spelling. Decide whether source locale is a per-record column, side-table field or an invariant of the model/project. Changing source locale requires a defined migration/validation policy.

### Admin workflow

- Source field shows a small, keyboard-accessible translation control.
- Activating it opens an accessible dialog with source text and one editor per enabled non-source language, including missing/complete state, validation and fallback preview.
- Translating an existing record is supported without rewriting the original source value.
- Record titles, breadcrumbs, list rows and relation picker labels resolve the selected locale when available; fall back to source value.
- The same record/field security and object scopes apply to reading and editing translations. Do not expose translations to unauthorized users through search, lookup or export.
- Translation writes are atomic with their audit record and use the same field contracts (length, format where applicable) as source fields. Specify optimistic locking and deletion behavior.

### Performance and API

- Admin lists load translations in batches using the visible record IDs, never one lookup per cell. Test 0/1/100/1000 rows and scope isolation.
- Document search, filtering, ordering and aggregation semantics: source-only until intentionally extended, or genuinely locale-aware with dedicated SQL/query support. Do not silently change ordering or pagination.
- Public APIs remain backward compatible by default; introduce deliberate locale selection via an explicit query parameter or `Accept-Language` and define response metadata/caching (`Vary`, ETag, cache keys). Data export/import must preserve translations without disclosing hidden fields.

## Implementation boundaries

1. **Catalog foundation:** pure lookup, embedded catalog loader, stable message IDs, interpolation, pluralization, locale selection, fallback and tests. Migrate existing admin catalog without removing translated content.
2. **English-first cleanup:** code comments/rustdoc, developer UI/CLI text, templates and docs. Keep migrations, code keys, fixtures and intentionally quoted German translation samples intact.
3. **Project label registry:** discover model/app metadata and build the Settings → Translations editor. Apply access control and deterministic key migrations.
4. **Record translation storage:** schema/migrations, source-locale policy, field metadata, atomic updates/audit, field dialog and localized record-name display.
5. **Locale-aware API and tooling:** explicit public API behavior, bulk loading, PO export/import, missing-key CI and language-contributor workflow.
6. **Release gate:** fresh crates.io consumer with offline EN/DE switching, SQLite/PostgreSQL integration tests, Windows/Linux and browser checks.

## Acceptance matrix

| Scenario | Expected |
| --- | --- |
| Fresh published Fexu project, no network access | English and every bundled, enabled catalog work immediately |
| Unsupported/missing UI translation | English fallback, no exposed raw key |
| Source name `Kerze` in a German-source record | Original retained; English/French variants can be added later |
| Switch language while viewing products | Localized record labels where available, otherwise source |
| Missing translations and 1000 records | Batch access; no N+1, no silent truncation or scope expansion |
| Cross-user/cross-model forged translation edit | Authorization failure, no write or information leak |
| Duplicate/invalid field/locale or long text | Validation failure, no partial saved content or audit |
| Source record deleted or renamed | Defined translation cleanup/key migration behavior |
| Existing 0.1.2 app and existing API client | Compatible unless explicit versioned migration documents otherwise |

## Existing implementation references

- `crates/fexu-admin/src/localization.rs`: built-in German/English tuple catalog.
- `crates/fexu-admin/src/html.rs`: admin language preference, record forms, authorization and auditing.
- `crates/fexu-admin/src/registry.rs`: model registration and admin metadata.
- `crates/fexu-admin/src/compose.rs`: transactional inline and related record writes.
- `crates/fexu-api/src/pages.rs`: application page rendering language.
- `crates/fexu-cli/src/text.rs`: CLI localization.
- `docs/i18n.md`: current supported scope; must be updated when this RFC is implemented.

Linked issues: [#148](https://github.com/Iam-Sanja/fexu-rs/issues/148) (tracker), [#84](https://github.com/Iam-Sanja/fexu-rs/issues/84) (forms), [#142](https://github.com/Iam-Sanja/fexu-rs/issues/142) (timezone), [#79](https://github.com/Iam-Sanja/fexu-rs/issues/79) (admin configuration).

## Decisions to resolve during first implementation PR

1. Separate core i18n module/crate vs internal shared service; public API stability.
2. PO/MO vs generated static catalogs and translation project version compatibility.
3. How auto-generated keys coexist with explicit app-defined IDs.
4. Where label overrides live, how they are imported/exported and how published app catalogs interact with project overrides.
5. Per-model vs per-record source locale and robust migration of existing content.
6. Extent and ordering of locale-aware search/sort/filters; document the initial supported subset.
7. Translation visibility in permissions/audits and project backup/dump workflows.

No changes to runtime behavior, database schema, publication state or deployed apps are made by accepting this RFC.
