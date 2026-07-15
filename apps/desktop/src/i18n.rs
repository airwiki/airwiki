//! Embedded user-interface localization with a small, explicit fallback chain.

use std::str::FromStr;

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use unic_langid::LanguageIdentifier;

const EN_US_SOURCE: &str = include_str!("../locales/en-US.ftl");
const ES_SOURCE: &str = include_str!("../locales/es.ftl");

#[cfg(test)]
pub(crate) const MESSAGE_IDS: &[&str] = &[
    "app-title",
    "nav-home",
    "nav-collections",
    "nav-review",
    "nav-wiki",
    "nav-search",
    "nav-integrations",
    "nav-devices",
    "nav-settings",
    "status-ready",
    "status-working",
    "status-needs-permission",
    "status-needs-attention",
    "status-optional-disabled",
    "dashboard-title",
    "dashboard-subtitle",
    "dashboard-all-ready",
    "component-local-ai",
    "component-collections",
    "component-review",
    "component-wiki",
    "component-lan",
    "component-chat",
    "component-background",
    "component-updates",
    "action-open",
    "action-configure",
    "action-review",
    "action-retry",
    "action-details",
    "action-copy",
    "onboarding-welcome-title",
    "onboarding-welcome-body",
    "onboarding-model-title",
    "onboarding-collection-title",
    "onboarding-lan-title",
    "onboarding-background-title",
    "onboarding-chat-title",
    "onboarding-complete-title",
    "onboarding-next",
    "onboarding-back",
    "onboarding-skip",
    "onboarding-finish",
    "search-placeholder",
    "search-coverage-federation-disabled",
    "search-coverage-offline-devices",
    "search-coverage-partial",
    "devices-title",
    "devices-searching",
    "devices-manual-advanced",
    "devices-this-address",
    "devices-manual-requires-lan",
    "primary-firewall-legacy-title",
    "primary-firewall-legacy-explanation",
    "peer-trust-trusted",
    "peer-activity-not-observed",
    "peer-activity-unavailable",
    "models-ready",
    "models-pending",
    "language-system",
    "language-spanish",
    "language-english",
    "tray-open",
    "tray-quit",
];

type Bundle = FluentBundle<FluentResource>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UiLocale {
    Es,
    EnUs,
}

impl UiLocale {
    pub(crate) fn from_system() -> Self {
        sys_locale::get_locale()
            .as_deref()
            .and_then(|locale| LanguageIdentifier::from_str(locale).ok())
            .filter(|locale| locale.language.as_str() == "es")
            .map_or(Self::EnUs, |_| Self::Es)
    }
}

pub(crate) struct Localization {
    current: UiLocale,
    en_us: Bundle,
    es: Bundle,
}

impl Localization {
    pub(crate) fn new(current: UiLocale) -> Result<Self, LocalizationError> {
        Ok(Self {
            current,
            en_us: build_bundle("en-US", EN_US_SOURCE)?,
            es: build_bundle("es", ES_SOURCE)?,
        })
    }

    pub(crate) fn set_locale(&mut self, locale: UiLocale) {
        self.current = locale;
    }

    pub(crate) fn text(&self, id: &str) -> String {
        self.text_with(id, None)
    }

    pub(crate) fn text_with(&self, id: &str, arguments: Option<&FluentArgs<'_>>) -> String {
        format_message(self.bundle(self.current), id, arguments)
            .or_else(|| format_message(&self.en_us, id, arguments))
            .unwrap_or_else(|| id.to_owned())
    }

    fn bundle(&self, locale: UiLocale) -> &Bundle {
        match locale {
            UiLocale::Es => &self.es,
            UiLocale::EnUs => &self.en_us,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum LocalizationError {
    #[error("invalid locale identifier `{0}`")]
    InvalidLocale(String),
    #[error("the {locale} translation catalog is invalid: {details}")]
    InvalidCatalog { locale: String, details: String },
}

fn build_bundle(locale: &str, source: &str) -> Result<Bundle, LocalizationError> {
    let language = LanguageIdentifier::from_str(locale)
        .map_err(|_| LocalizationError::InvalidLocale(locale.to_owned()))?;
    let resource = FluentResource::try_new(source.to_owned()).map_err(|(_, errors)| {
        LocalizationError::InvalidCatalog {
            locale: locale.to_owned(),
            details: errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        }
    })?;
    let mut bundle = FluentBundle::new(vec![language]);
    bundle
        .add_resource(resource)
        .map_err(|errors| LocalizationError::InvalidCatalog {
            locale: locale.to_owned(),
            details: errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        })?;
    Ok(bundle)
}

fn format_message(bundle: &Bundle, id: &str, arguments: Option<&FluentArgs<'_>>) -> Option<String> {
    let message = bundle.get_message(id)?;
    let pattern = message.value()?;
    let mut errors = Vec::new();
    let value = bundle.format_pattern(pattern, arguments, &mut errors);
    errors.is_empty().then(|| value.into_owned())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    #[test]
    fn every_required_message_exists_in_both_catalogs() {
        let localization = Localization::new(UiLocale::EnUs).unwrap();
        let missing = MESSAGE_IDS
            .iter()
            .filter(|id| {
                localization
                    .en_us
                    .get_message(id)
                    .and_then(|message| message.value())
                    .is_none()
                    || localization
                        .es
                        .get_message(id)
                        .and_then(|message| message.value())
                        .is_none()
            })
            .copied()
            .collect::<Vec<_>>();

        assert!(missing.is_empty(), "missing translations: {missing:?}");
    }

    #[test]
    fn english_is_used_when_a_message_is_missing_from_the_selected_catalog() {
        let localization = Localization::new(UiLocale::Es).unwrap();

        assert_eq!(localization.text("app-title"), "AirWiki");
    }

    #[test]
    fn unknown_message_returns_its_stable_identifier() {
        let localization = Localization::new(UiLocale::EnUs).unwrap();

        assert_eq!(localization.text("unknown-message"), "unknown-message");
    }

    #[test]
    fn catalogs_have_the_same_messages_and_parameters() {
        assert_eq!(catalog_shape(EN_US_SOURCE), catalog_shape(ES_SOURCE));
    }

    #[test]
    fn catalogs_use_separate_peer_trust_and_activity_messages() {
        for catalog in [EN_US_SOURCE, ES_SOURCE] {
            assert!(!catalog.contains("peer-state-trusted"));
            assert!(!catalog.contains("peer-state-offline"));
            assert!(catalog.contains("peer-trust-trusted"));
            assert!(catalog.contains("peer-activity-not-observed"));
        }
    }

    fn catalog_shape(source: &str) -> BTreeMap<String, BTreeSet<String>> {
        let mut messages = BTreeMap::<String, String>::new();
        let mut current = None;
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if !line.starts_with(char::is_whitespace)
                && let Some((id, value)) = trimmed.split_once('=')
            {
                let id = id.trim().to_owned();
                messages.insert(id.clone(), value.trim().to_owned());
                current = Some(id);
                continue;
            }
            if let Some(id) = current.as_ref()
                && let Some(value) = messages.get_mut(id)
            {
                value.push(' ');
                value.push_str(trimmed);
            }
        }

        messages
            .into_iter()
            .map(|(id, value)| (id, fluent_parameters(&value)))
            .collect()
    }

    fn fluent_parameters(value: &str) -> BTreeSet<String> {
        value
            .split('$')
            .skip(1)
            .filter_map(|suffix| {
                let parameter = suffix
                    .chars()
                    .take_while(|character| {
                        character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                    })
                    .collect::<String>();
                (!parameter.is_empty()).then_some(parameter)
            })
            .collect()
    }
}
