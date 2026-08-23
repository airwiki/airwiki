//! Embedded localization shared with the Tauri webview.

use std::str::FromStr;

use fluent_bundle::{FluentBundle, FluentResource};
use unic_langid::LanguageIdentifier;

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
    use super::{Localization, UiLocale};

    #[test]
    fn native_tray_messages_exist_in_both_catalogs() -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (UiLocale::EnUs, "Open AirWiki", "Quit completely"),
            (UiLocale::Es, "Abrir AirWiki", "Salir completamente"),
        ];
        for (locale, expected_open, expected_quit) in cases {
            let localization = Localization::new(locale)?;
            assert_eq!(
                localization.text("tray-open").as_deref(),
                Some(expected_open)
            );
            assert_eq!(
                localization.text("tray-quit").as_deref(),
                Some(expected_quit)
            );
        }
        Ok(())
    }
}
