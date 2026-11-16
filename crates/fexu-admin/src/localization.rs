//! Embedded gettext-compatible admin catalogs, with English as the canonical fallback.
//! Locale input is resolved against a fixed allowlist, never used as a filesystem path.

use serde_json::{Map, Value};

const EN_PO: &str = include_str!("../locales/en.po");
const DE_PO: &str = include_str!("../locales/de.po");

pub fn normalize(requested: &str, default_locale: &str, fallback_locale: &str) -> String {
    match requested {
        "de" | "en" => requested.to_owned(),
        _ if default_locale == "de" || default_locale == "en" => default_locale.to_owned(),
        _ if fallback_locale == "de" || fallback_locale == "en" => fallback_locale.to_owned(),
        _ => "en".to_owned(),
    }
}

/// Parses the fixed three-field PO entries shipped with this crate.
/// This is not a general-purpose gettext parser; catalog generation validates its input.
fn parse_catalog(source: &str) -> Map<String, Value> {
    let mut map = Map::new();
    let mut key: Option<String> = None;
    let mut translation: Option<String> = None;
    for line in source.lines().chain(std::iter::once("")) {
        if line.is_empty() {
            if let (Some(key), Some(translation)) = (key.take(), translation.take()) {
                assert!(map.insert(key, Value::String(translation)).is_none(), "duplicate admin translation key");
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("msgctxt ") {
            key = Some(serde_json::from_str(value).expect("valid PO context"));
        } else if let Some(value) = line.strip_prefix("msgstr ") {
            translation = Some(serde_json::from_str(value).expect("valid PO translation"));
        }
    }
    map
}

pub fn catalog(locale: &str) -> Value {
    let english = parse_catalog(EN_PO);
    if locale != "de" {
        return Value::Object(english);
    }
    let mut translated = english;
    for (key, value) in parse_catalog(DE_PO) {
        if value.as_str().is_some_and(|text| !text.is_empty()) && translated.contains_key(&key) {
            translated.insert(key, value);
        }
    }
    Value::Object(translated)
}

#[cfg(test)]
mod catalog_tests {
    use super::{catalog, normalize, parse_catalog, DE_PO, EN_PO};

    #[test]
    fn bundled_catalogs_have_matching_keys_and_preserve_existing_labels() {
        let en = parse_catalog(EN_PO);
        let de = parse_catalog(DE_PO);
        assert_eq!(en.len(), 313);
        assert_eq!(en.keys().collect::<Vec<_>>(), de.keys().collect::<Vec<_>>());
        assert_eq!(en["save"], "Save");
        assert_eq!(de["save"], "Speichern");
        assert_eq!(en["menu"], "Menu");
        assert_eq!(de["menu"], "Menü");
        assert_eq!(catalog("en")["login"], "Sign in");
        assert_eq!(catalog("de")["login"], "Anmelden");
    }

    #[test]
    fn unknown_language_falls_back_to_english_without_file_access() {
        assert_eq!(catalog("fr")["save"], "Save");
        assert_eq!(normalize("fr", "en", "de"), "en");
        assert_eq!(normalize("fr", "fr", "fr"), "en");
        assert_eq!(normalize("de", "en", "en"), "de");
    }
}


pub fn notice(locale: &str, code: &str) -> Option<String> {
    let catalog = catalog(locale);
    let key = match code {
        "created" => "notice_created",
        "saved" => "notice_saved",
        "deleted" => "notice_deleted",
        "updated" => "notice_updated",
        "changed" => "notice_updated",
        _ => return None,
    };
    catalog
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

pub fn bulk_notice(locale: &str, count: u32) -> String {
    if locale == "de" {
        format!("{count} Datensätze wurden geändert.")
    } else {
        format!("{count} records were changed.")
    }
}

pub fn validation_text(locale: &str, detail: &str) -> String {
    let german = locale == "de";
    if detail.contains("must not be blank") || detail.contains("validation.null") {
        return if german {
            "Pflichtfeld.".into()
        } else {
            "Required.".into()
        };
    }
    if detail.contains("must have at least") || detail.contains("validation.min_length") {
        return if german {
            "Der Wert ist zu kurz.".into()
        } else {
            "The value is too short.".into()
        };
    }
    if detail.contains("may have at most") || detail.contains("validation.max_length") {
        return if german {
            "Der Wert ist zu lang.".into()
        } else {
            "The value is too long.".into()
        };
    }
    if detail.contains("not writable") {
        return if german {
            "Dieses Feld kann nicht gesetzt werden.".into()
        } else {
            "This field cannot be set.".into()
        };
    }
    if detail.contains("validation.date") {
        return catalog(locale)["validation_date"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.time") {
        return catalog(locale)["validation_time"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.datetime") {
        return catalog(locale)["validation_datetime"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.min_value") || detail.contains("must be >=") {
        return fexu_core::message_text(locale, "validation.min_value").to_owned();
    }
    if detail.contains("validation.email") || detail.contains("must be an email") {
        return fexu_core::message_text(locale, "validation.email").to_owned();
    }
    if detail.contains("validation.slug") || detail.contains("lowercase slug") {
        return fexu_core::message_text(locale, "validation.slug").to_owned();
    }
    if detail.contains("validation.choice") {
        return fexu_core::message_text(locale, "validation.choice").to_owned();
    }
    if detail.contains("validation.postal") {
        return fexu_core::message_text(locale, "validation.postal").to_owned();
    }
    if detail.contains("validation.required") {
        return if german {
            "Pflichtfeld.".into()
        } else {
            "Required.".into()
        };
    }
    if detail.contains("validation.inline_max") {
        return catalog(locale)["inline_limit"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.inline_duplicate") {
        return if german {
            "Eine Zeile ist doppelt.".into()
        } else {
            "A row is listed twice.".into()
        };
    }
    if detail.contains("validation.inline_foreign") || detail.contains("validation.readonly") {
        return catalog(locale)["inline_foreign"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.inline_field") {
        return if german {
            "Eine Zeile enthält ein ungültiges Feld.".into()
        } else {
            "A row contains an invalid field.".into()
        };
    }
    if detail.contains("conflict.stale") {
        return catalog(locale)["inline_conflict"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("admin.relation.size") {
        return catalog(locale)["m2m_size"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("admin.store") {
        return if german {
            "Die Datei konnte nicht gespeichert werden.".into()
        } else {
            "The file could not be stored.".into()
        };
    }
    if detail.contains("admin.relation") {
        return catalog(locale)["m2m_invalid"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail.contains("validation.inline_failed") {
        return catalog(locale)["inline_failed"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    if detail == "Invalid email or password" {
        return catalog(locale)["invalid_credentials"]
            .as_str()
            .unwrap_or(detail)
            .to_owned();
    }
    detail.to_owned()
}
