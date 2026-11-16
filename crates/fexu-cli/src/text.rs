//! Central CLI messages from bundled English and German PO catalogs.
//! English is the default developer-facing language; German is an explicit
//! `FEXU_LOCALE=de` override, independent of the application's UI locale.

use fexu_core::i18n::PoCatalog;
use std::sync::OnceLock;

const EN_PO: &str = include_str!("../locales/en.po");
const DE_PO: &str = include_str!("../locales/de.po");

fn catalogs() -> &'static (PoCatalog, PoCatalog) {
    static CATALOGS: OnceLock<(PoCatalog, PoCatalog)> = OnceLock::new();
    CATALOGS.get_or_init(|| {
        let english = PoCatalog::parse(EN_PO).expect("valid bundled English CLI catalog");
        let german = PoCatalog::parse(DE_PO).expect("valid bundled German CLI catalog");
        german.check_keys(&english).expect("CLI catalog keys match");
        (english, german)
    })
}

pub fn line(key: &str) -> &'static str {
    line_for(&locale(), key)
}

fn line_for(locale: &str, key: &str) -> &'static str {
    let (english, german) = catalogs();
    let translation = if locale == "de" { Some(german) } else { None };
    // Messages resolve to storage owned by a static OnceLock.
    PoCatalog::resolve(english, translation, &format!("cli.{key}")).unwrap_or("missing message")
}

pub fn locale() -> String {
    if let Ok(value) = std::env::var("FEXU_LOCALE") {
        if let Some(locale) = supported(&value) {
            return locale;
        }
    }
    // CLI messages do not inherit the application's UI language.
    "en".to_owned()
}

fn supported(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "de" => Some("de".to_owned()),
        "en" => Some("en".to_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{catalogs, line_for};

    #[test]
    fn all_cli_messages_are_bundled_in_both_languages() {
        let (english, german) = catalogs();
        assert_eq!(english.len(), 16);
        assert_eq!(german.len(), 16);
        german.check_keys(english).unwrap();
        for key in [
            "project_created",
            "model_created",
            "dev_started",
            "dev_stop_hint",
            "dev_stopped",
            "dev_test_stopped",
            "admin_assets_missing",
            "user_created",
            "flag_active",
            "flag_staff_on",
            "flag_staff_off",
            "flag_super_on",
            "flag_super_off",
            "super_without_staff",
            "dev_no_migrate",
            "port_busy",
        ] {
            assert_ne!(line_for("en", key), "missing message", "{key}");
            assert_ne!(line_for("de", key), "missing message", "{key}");
        }
    }

    #[test]
    fn existing_english_and_german_outputs_are_unchanged() {
        assert_eq!(line_for("en", "model_created"), "Created model:");
        assert_eq!(line_for("de", "model_created"), "Modell erzeugt:");
        assert_eq!(line_for("fr", "model_created"), "Created model:");
        assert_eq!(line_for("de", "super_without_staff"), "Hinweis: --superuser ohne --staff. Die Modellrechte gelten, der Admin-Zugang bleibt zu.");
        assert_eq!(line_for("en", "flag_staff_off"), "Staff: no. Without staff the admin stays closed.");
        assert!(!line_for("de", "flag_super_on").contains("is_staff = true"));
        assert_eq!(line_for("en", "missing.key"), "missing message");
    }
}
