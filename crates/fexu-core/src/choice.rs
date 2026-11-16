//! Stable stored values for Rust enums.
//!
//! Das Derive `FexuChoice` setzt `VALUES`, `stored` und `from_stored`.
//! Unknown values are rejected in JSON and before writes. Removing a
//! stored value requires a data migration: existing rows are not rewritten.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChoiceValue {
    pub value: &'static str,
    pub label_de: &'static str,
    pub label_en: &'static str,
}

pub trait FexuChoice: Copy + Sized + Send + Sync + 'static {
    const VALUES: &'static [ChoiceValue];

    fn stored(self) -> &'static str;

    fn from_stored(value: &str) -> Option<Self>;

    fn label(self, locale: &str) -> &'static str {
        let stored = self.stored();
        Self::VALUES
            .iter()
            .find(|choice| choice.value == stored)
            .map(|choice| {
                if locale == "de" {
                    choice.label_de
                } else {
                    choice.label_en
                }
            })
            .unwrap_or(stored)
    }
}

pub fn choice_label(choices: &[ChoiceValue], locale: &str, stored: &str) -> String {
    choices
        .iter()
        .find(|choice| choice.value == stored)
        .map(|choice| {
            if locale == "de" {
                choice.label_de
            } else {
                choice.label_en
            }
            .to_owned()
        })
        .unwrap_or_else(|| stored.to_owned())
}

pub fn choice_is_known(choices: &[ChoiceValue], stored: &str) -> bool {
    choices.iter().any(|choice| choice.value == stored)
}
