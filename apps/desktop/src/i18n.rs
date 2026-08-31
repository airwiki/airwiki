//! Embedded localization shared with the Tauri webview.

use std::str::FromStr;

use fluent_bundle::{FluentBundle, FluentResource};
use unic_langid::LanguageIdentifier;

use crate::model_config::LocalePreference;

const EN_US_SOURCE: &str = include_str!("../locales/en-US.ftl");
const ES_SOURCE: &str = include_str!("../locales/es.ftl");

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

    const fn tag(self) -> &'static str {
        match self {
            Self::Es => "es",
            Self::EnUs => "en-US",
        }
    }

    const fn source(self) -> &'static str {
        match self {
            Self::Es => ES_SOURCE,
            Self::EnUs => EN_US_SOURCE,
        }
    }
}

impl From<LocalePreference> for UiLocale {
    fn from(value: LocalePreference) -> Self {
        match value {
            LocalePreference::System => Self::from_system(),
            LocalePreference::Es => Self::Es,
            LocalePreference::En => Self::EnUs,
        }
    }
}

pub(crate) struct Localization {
    bundle: Bundle,
}

impl Localization {
    pub(crate) fn new(locale: UiLocale) -> Result<Self, LocalizationError> {
        let language = LanguageIdentifier::from_str(locale.tag())
            .map_err(|_| LocalizationError::InvalidLocale)?;
        let resource = FluentResource::try_new(locale.source().to_owned())
            .map_err(|_| LocalizationError::InvalidCatalog)?;
        let mut bundle = FluentBundle::new(vec![language]);
        bundle
            .add_resource(resource)
            .map_err(|_| LocalizationError::InvalidCatalog)?;
        Ok(Self { bundle })
    }

    pub(crate) fn text(&self, id: &str) -> Option<String> {
        let message = self.bundle.get_message(id)?;
        let pattern = message.value()?;
        let mut errors = Vec::new();
        let value = self.bundle.format_pattern(pattern, None, &mut errors);
        errors.is_empty().then(|| value.into_owned())
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum LocalizationError {
    #[error("invalid embedded locale")]
    InvalidLocale,
    #[error("invalid embedded translation catalog")]
    InvalidCatalog,
}

#[cfg(test)]
mod tests {
    use super::{LocalePreference, Localization, UiLocale};

    #[test]
    fn native_menu_messages_exist_in_both_catalogs() -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (
                UiLocale::EnUs,
                "Open AirWiki",
                "Quit completely",
                [
                    "File",
                    "Edit",
                    "View",
                    "Window",
                    "New wiki…",
                    "Library",
                    "Search",
                    "Settings",
                    "Quit AirWiki",
                    "Exit AirWiki",
                ],
            ),
            (
                UiLocale::Es,
                "Abrir AirWiki",
                "Salir completamente",
                [
                    "Archivo",
                    "Edición",
                    "Visualización",
                    "Ventana",
                    "Nueva wiki…",
                    "Biblioteca",
                    "Buscar",
                    "Configuración",
                    "Salir de AirWiki",
                    "Salir de AirWiki",
                ],
            ),
        ];
        let menu_ids = [
            "native-menu-file",
            "native-menu-edit",
            "native-menu-view",
            "native-menu-window",
            "native-menu-new-wiki",
            "native-menu-library",
            "native-menu-search",
            "native-menu-settings",
            "native-menu-quit",
            "native-menu-exit",
        ];
        for (locale, expected_open, expected_quit, expected_menu) in cases {
            let localization = Localization::new(locale)?;
            assert_eq!(
                localization.text("tray-open").as_deref(),
                Some(expected_open)
            );
            assert_eq!(
                localization.text("tray-quit").as_deref(),
                Some(expected_quit)
            );
            for (id, expected) in menu_ids.iter().zip(expected_menu) {
                assert_eq!(localization.text(id).as_deref(), Some(expected));
            }
        }
        Ok(())
    }

    #[test]
    fn explicit_airwiki_language_preferences_select_the_matching_native_locale() {
        assert_eq!(UiLocale::from(LocalePreference::Es), UiLocale::Es);
        assert_eq!(UiLocale::from(LocalePreference::En), UiLocale::EnUs);
    }
}
