//! Shared, embedded gettext-compatible message catalogs.
//!
//! Catalogs are parsed from bundled source text once. Locale values never select
//! filesystem paths or trigger network access. English is the source fallback.

use std::collections::BTreeMap;

/// A catalog with stable, namespaced `msgctxt` identifiers.
///
/// This reader intentionally accepts the subset used by Fexu's shipped UI
/// catalogs: `msgctxt`, `msgid`, `msgstr`, multiline quoted strings, and
/// gettext metadata headers. Plural entries are rejected until explicitly
/// supported, rather than silently mistranslated.
#[derive(Debug, Default)]
pub struct PoCatalog {
    entries: BTreeMap<String, String>,
}

impl PoCatalog {
    pub fn parse(source: &str) -> Result<Self, String> {
        #[derive(Default)]
        struct Entry {
            context: Option<String>,
            id: Option<String>,
            translated: Option<String>,
        }
        #[derive(Clone, Copy)]
        enum Part {
            Context,
            Id,
            Translation,
        }
        fn quoted(input: &str, line: usize) -> Result<String, String> {
            if !input.starts_with('"') {
                return Err(format!("PO line {line}: expected quoted text"));
            }
            serde_json::from_str::<String>(input)
                .map_err(|_| format!("PO line {line}: invalid quoted string"))
        }
        fn finish(
            entry: &mut Entry,
            map: &mut BTreeMap<String, String>,
            line: usize,
        ) -> Result<(), String> {
            if entry.id.is_none() && entry.context.is_none() && entry.translated.is_none() {
                return Ok(());
            }
            // The conventional gettext header has no context and an empty msgid.
            if entry.context.is_none() && entry.id.as_deref() == Some("") {
                *entry = Entry::default();
                return Ok(());
            }
            let context = entry
                .context
                .take()
                .ok_or_else(|| format!("PO line {line}: missing msgctxt"))?;
            let id = entry
                .id
                .take()
                .ok_or_else(|| format!("PO line {line}: missing msgid"))?;
            let translated = entry
                .translated
                .take()
                .ok_or_else(|| format!("PO line {line}: missing msgstr"))?;
            if context.trim().is_empty() || id.is_empty() {
                return Err(format!("PO line {line}: empty translation key or source text"));
            }
            if map.insert(context.clone(), translated).is_some() {
                return Err(format!("PO line {line}: duplicate key {context}"));
            }
            Ok(())
        }
        let mut entries = BTreeMap::new();
        let mut entry = Entry::default();
        let mut current: Option<Part> = None;
        for (index, raw) in source.lines().chain(std::iter::once("")).enumerate() {
            let line = index + 1;
            let text = raw.trim();
            if text.is_empty() {
                finish(&mut entry, &mut entries, line)?;
                current = None;
                continue;
            }
            if text.starts_with('#') {
                continue;
            }
            if text.starts_with("msgid_plural ") || text.starts_with("msgstr[") {
                return Err(format!("PO line {line}: plural forms are not yet supported"));
            }
            let (part, value) = if let Some(rest) = text.strip_prefix("msgctxt ") {
                (Part::Context, rest)
            } else if let Some(rest) = text.strip_prefix("msgid ") {
                (Part::Id, rest)
            } else if let Some(rest) = text.strip_prefix("msgstr ") {
                (Part::Translation, rest)
            } else if text.starts_with('"') {
                let part = current.ok_or_else(|| format!("PO line {line}: orphan continuation"))?;
                let decoded = quoted(text, line)?;
                let target = match part {
                    Part::Context => &mut entry.context,
                    Part::Id => &mut entry.id,
                    Part::Translation => &mut entry.translated,
                };
                target
                    .as_mut()
                    .ok_or_else(|| format!("PO line {line}: missing preceding value"))?
                    .push_str(&decoded);
                continue;
            } else {
                return Err(format!("PO line {line}: unexpected directive"));
            };
            let decoded = quoted(value, line)?;
            let target = match part {
                Part::Context => &mut entry.context,
                Part::Id => &mut entry.id,
                Part::Translation => &mut entry.translated,
            };
            if target.replace(decoded).is_some() {
                return Err(format!("PO line {line}: duplicate directive"));
            }
            current = Some(part);
        }
        Ok(Self { entries })
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str).filter(|value| !value.is_empty())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Resolves the requested language, falling back to the English source
    /// catalog. Unknown keys never expose raw message IDs to users.
    pub fn resolve<'a>(
        english: &'a Self,
        translated: Option<&'a Self>,
        key: &str,
    ) -> Option<&'a str> {
        translated
            .and_then(|catalog| catalog.get(key))
            .or_else(|| english.get(key))
    }

    /// Validates that a translated catalog has no extra keys and that all
    /// English keys are represented (empty translations may use fallback).
    pub fn check_keys(&self, english: &Self) -> Result<(), String> {
        for key in english.entries.keys() {
            if !self.entries.contains_key(key) {
                return Err(format!("translation missing key: {key}"));
            }
        }
        for key in self.entries.keys() {
            if !english.entries.contains_key(key) {
                return Err(format!("translation has unknown key: {key}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::PoCatalog;

    #[test]
    fn parses_contexts_multiline_text_and_english_fallback() {
        let english = r#"msgid ""
msgstr ""
"Language: en\n"

msgctxt "app.save"
msgid "Save"
msgstr "Save"

msgctxt "app.more"
msgid "More"
msgstr "More"
"#;
        let german = r#"msgctxt "app.save"
msgid "Save"
msgstr "Spei"
"chern"

msgctxt "app.more"
msgid "More"
msgstr ""
"#;
        let en = PoCatalog::parse(english).unwrap();
        let de = PoCatalog::parse(german).unwrap();
        de.check_keys(&en).unwrap();
        assert_eq!(
            PoCatalog::resolve(&en, Some(&de), "app.save"),
            Some("Speichern")
        );
        assert_eq!(PoCatalog::resolve(&en, Some(&de), "app.more"), Some("More"));
        assert_eq!(PoCatalog::resolve(&en, None, "app.save"), Some("Save"));
        assert_eq!(PoCatalog::resolve(&en, Some(&de), "unknown"), None);
    }

    #[test]
    fn rejects_broken_or_ambiguous_catalogs() {
        let duplicate = r#"msgctxt "a"
msgid "A"
msgstr "A"

msgctxt "a"
msgid "A"
msgstr "Again"
"#;
        assert!(PoCatalog::parse(duplicate)
            .unwrap_err()
            .contains("duplicate key"));
        assert!(PoCatalog::parse("msgctxt \"a\"\nmsgid \"A\"\nmsgstr[0] \"A\"")
            .is_err());
        assert!(PoCatalog::parse("msgctxt \"a\"\nmsgid \"A\"\nmsgstr \"broken")
            .is_err());
    }
}
