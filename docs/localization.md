# Localization in the admin

[Overview](index.md) · Architecture: [i18n.md](i18n.md)

## Formats

The HTML admin formats numbers in `display_cell`: German uses periods for digit grouping, English uses commas. Fields ending in `_cents` use the project currency and two decimal places; stored values remain integer cents. Date values retain their database representation. The server does not call `Intl`.

## Language of the homepage

`negotiate_locale` in `crates/fexu-api/src/pages.rs` checks in this order:

1. `?lang=` only if the value is `de` or `en`, optionally with region such as `de-DE`.
2. The first supported entry of `Accept-Language`, weighted with `q`.
3. `project.default_locale`.
4. `project.fallback_locale`, otherwise `en`.

The selected language is added to the Tera context as `locale`; it never becomes a file path. Tera escapes rendered text. Generated projects link to `/?lang=de` and `/?lang=en`.

## CLI

`FEXU_LOCALE=en` or `de` selects the CLI catalog. When `FEXU_LOCALE` is unset, `crates/fexu-cli/src/text.rs` uses English and does not inherit the application UI locale. Project-generation, model-generation and development-server start/stop messages come from that catalog. The generated `fexu dev` program emits the English startup line `Fexu listening at http://...`. `fexu migrate make` prints `No schema changes. No migration was written.` when the schema is unchanged. `fexu migrate status` prints English headings such as `Auth migrations` unless the locale is `de`, in which case the heading is `Auth-Migrationen`. The stored states stay `angewendet` and `ausstehend`. A locale other than `de` displays those states as `applied` and `pending`. The sentences `Angewendet` and `Keine Schemaänderung` are not in the source tree.

## Tests

`examples/myshop/tests/admin_html.rs` checks the German login page, `Accept-Language: en` and switching to English. Tests in `crates/fexu-api/src/pages.rs` check English fallback and invalid locales. The admin registry tests reject invalid branding colors.
