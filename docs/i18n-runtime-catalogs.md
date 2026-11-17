# Shared runtime message catalogs (first implementation)

> Implementation scope: first shared-catalog PR for [#148](https://github.com/Iam-Sanja/fexu-rs/issues/148). This is not yet complete project/model/record translation.

## Scope and location

- `crates/fexu-core/src/i18n.rs`: reusable `PoCatalog` parser, key consistency validation, and English fallback. This is in `fexu-core` to avoid adding an extra public crate to the existing 16-package release cycle or creating dependency cycles.
- `crates/fexu-cli/locales/{en,de}.po`: 16 existing CLI messages. The CLI continues to return `&'static str` and defaults to English; `FEXU_LOCALE=de` still selects the bundled German catalog. Unsupported locale codes fall back to English.
- `crates/fexu-core/locales/{en,de}.po`: 31 validation, form, and request display messages. Existing error keys and the `message_text(locale, code) -> &'static str` API are preserved.
- Only officially bundled languages are supported; catalogs are embedded with `include_str!`, parsed once and then read from immutable process memory. There is no runtime catalog download or filesystem path resolution.

The catalog reader accepts contextual gettext messages (`msgctxt`, `msgid`, `msgstr`), header metadata and continued quoted strings; it rejects malformed entries, duplicate keys and unsupported plural directives. Plural handling and more complete gettext grammar require a dedicated extension before translated plural messages are introduced. PO translation projects can later provide reviewed catalog snapshots for release packaging.

## Unchanged contract

- Error codes stay language-independent; source text defaults to English.
- Existing explicit German outputs must remain byte-for-byte equivalent. Unknown requested languages return English source strings.
- No database, route, API schema, CLI command names, or user data is changed.
- Legacy `FieldOverride` remains compatible, including explicitly supplied German/English strings. A separate design must cover dynamic project-defined field overrides before additional locales can be applied there.

## Follow-up

- Migrate the admin's PO reader from PR #150 to the shared `PoCatalog` service, after #150 is merged; do not maintain duplicate engines indefinitely.
- Inventory other hard-coded user-visible strings beyond the CLI/validation catalog, prioritize auth/HTML and generated applications, and distinguish deliberately English-only developer diagnostics.
- Add catalog extraction, placeholder/plural quality checks and a translator contribution workflow; ship additional approved language catalogs in future releases.
- Implement project-level label translations and per-record field translations separately under #148.

## Verification

- Rust unit tests cover parsing/duplicate rejection/multiline strings, catalog key matching, fallback, and preserved messages.
- Required GitHub CI remains the merge gate. No claim of successful local cargo compilation is made solely from reviewing this document.
