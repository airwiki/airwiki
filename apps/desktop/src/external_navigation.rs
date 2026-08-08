//! Narrow native boundary for opening user-confirmed web links.
//!
//! The caller supplies a validated HTTP(S) URL, never a command, path or
//! operating-system URI. Platform failures are intentionally sanitized.

use tauri::Url;
use thiserror::Error;

const MAX_EXTERNAL_URL_BYTES: usize = 2_048;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternalNavigationError {
    #[error("the external URL is not allowed")]
    InvalidUrl,
    #[error("external navigation is unsupported")]
    Unsupported,
    #[error("the external destination could not be opened")]
    OpenFailed,
}

pub(crate) fn validate_external_url(value: &str) -> Result<Url, ExternalNavigationError> {
    if value.is_empty()
        || value.len() > MAX_EXTERNAL_URL_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(ExternalNavigationError::InvalidUrl);
    }
    let url = Url::parse(value).map_err(|_| ExternalNavigationError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ExternalNavigationError::InvalidUrl);
    }
    Ok(url)
}

pub(crate) fn open_external_url(url: &Url) -> Result<(), ExternalNavigationError> {
    platform::open(url)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{ExternalNavigationError, Url};

    pub(super) fn open(url: &Url) -> Result<(), ExternalNavigationError> {
        let status = std::process::Command::new("/usr/bin/open")
            .arg("--")
            .arg(url.as_str())
            .status()
            .map_err(|_| ExternalNavigationError::OpenFailed)?;
        status
            .success()
            .then_some(())
            .ok_or(ExternalNavigationError::OpenFailed)
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::{ExternalNavigationError, Url};

    pub(super) fn open(url: &Url) -> Result<(), ExternalNavigationError> {
        let status = std::process::Command::new("rundll32.exe")
            .arg("url.dll,FileProtocolHandler")
            .arg(url.as_str())
            .status()
            .map_err(|_| ExternalNavigationError::OpenFailed)?;
        status
            .success()
            .then_some(())
            .ok_or(ExternalNavigationError::OpenFailed)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::{ExternalNavigationError, Url};

    pub(super) fn open(_url: &Url) -> Result<(), ExternalNavigationError> {
        Err(ExternalNavigationError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_credential_free_http_urls_are_accepted() {
        assert!(validate_external_url("https://licenses.example.test/model?q=1#terms").is_ok());
        assert!(validate_external_url("http://example.test/notice").is_ok());
        assert!(validate_external_url("file:///tmp/private").is_err());
        assert!(validate_external_url("mailto:person@example.test").is_err());
        assert!(validate_external_url("https://secret@example.test/terms").is_err());
        assert!(validate_external_url("https://example.test/\ncommand").is_err());
    }
}
