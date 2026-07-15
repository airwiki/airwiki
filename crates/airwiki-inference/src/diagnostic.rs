use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sysinfo::{Disks, System};

use crate::catalog::{GIB, INSTALL_HEADROOM_BYTES};

const REQUIRED_RAM_BYTES: u64 = 8 * GIB;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareReport {
    pub os: String,
    pub architecture: String,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub available_disk_bytes: u64,
    pub avx2: bool,
    pub metal_available: bool,
    pub supported_target: bool,
    pub can_install: bool,
    pub issues: Vec<String>,
}

pub fn diagnose_hardware(data_dir: &Path) -> Result<HardwareReport> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("cannot create data directory {}", data_dir.display()))?;

    let mut system = System::new_all();
    system.refresh_all();
    let total_memory_bytes = system.total_memory();
    let available_memory_bytes = system.available_memory();
    let available_disk_bytes = available_space_for(data_dir)?;
    let avx2 = avx2_available();
    // Every supported Apple Silicon Mac exposes Metal. Keeping the value explicit lets model
    // selection and future diagnostics operate on injected reports in tests.
    let metal_available = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    let supported_target = cfg!(all(target_os = "macos", target_arch = "aarch64"))
        || cfg!(all(target_os = "windows", target_arch = "x86_64"));

    let mut issues = Vec::new();
    if !supported_target {
        issues.push(format!(
            "MVP installers support macOS arm64 and Windows x64; this build is {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    }
    if available_disk_bytes < INSTALL_HEADROOM_BYTES {
        issues.push(format!(
            "At least 1 GiB of installation headroom is required; {:.1} GiB is available",
            available_disk_bytes as f64 / 1024_f64.powi(3)
        ));
    }
    if supported_target && total_memory_bytes < REQUIRED_RAM_BYTES {
        issues.push(format!(
            "Supported local models require at least 8 GiB RAM; {:.1} GiB was detected",
            total_memory_bytes as f64 / 1024_f64.powi(3)
        ));
    }
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) && !avx2 {
        issues.push("The Windows CPU must support AVX2".to_owned());
    }

    Ok(HardwareReport {
        os: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        total_memory_bytes,
        available_memory_bytes,
        available_disk_bytes,
        avx2,
        metal_available,
        supported_target,
        can_install: issues.is_empty(),
        issues,
    })
}

pub(crate) fn available_space_for(path: &Path) -> Result<u64> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let disks = Disks::new_with_refreshed_list();
    disks
        .iter()
        .filter(|disk| path_is_on_mount(&canonical, disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(|disk| disk.available_space())
        .context("could not determine available disk space")
}

fn path_is_on_mount(path: &Path, mount: &Path) -> bool {
    #[cfg(target_os = "windows")]
    {
        windows_path_is_on_mount(&path.to_string_lossy(), &mount.to_string_lossy())
    }
    #[cfg(not(target_os = "windows"))]
    {
        path.starts_with(mount)
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug, PartialEq, Eq)]
struct WindowsAbsolutePath {
    volume: String,
    components: Vec<String>,
}

/// Compares canonical Windows paths without relying on their presentation.
///
/// `std::fs::canonicalize` normally adds the verbatim `\\?\` prefix while
/// `sysinfo` reports ordinary drive or UNC mount paths. This parser accepts
/// both forms, treats separators and component case according to normal
/// Windows filesystem semantics, and compares whole components rather than
/// vulnerable string prefixes. Drive-letter and UNC roots are supported;
/// other device namespaces deliberately do not match a disk.
#[cfg(any(target_os = "windows", test))]
fn windows_path_is_on_mount(path: &str, mount: &str) -> bool {
    let (Some(path), Some(mount)) = (
        parse_windows_absolute_path(path),
        parse_windows_absolute_path(mount),
    ) else {
        return false;
    };
    path.volume == mount.volume && path.components.starts_with(&mount.components)
}

#[cfg(any(target_os = "windows", test))]
fn parse_windows_absolute_path(raw: &str) -> Option<WindowsAbsolutePath> {
    let normalized = raw.replace('\\', "/");
    let normalized = if let Some(rest) = strip_prefix_ascii_case(&normalized, "//?/UNC/") {
        format!("//{rest}")
    } else if let Some(rest) = strip_prefix_ascii_case(&normalized, "//?/") {
        rest.to_owned()
    } else {
        normalized
    };
    if normalized.starts_with("//./") {
        return None;
    }

    if let Some(rest) = normalized.strip_prefix("//") {
        let mut parts = rest.split('/').filter(|part| !part.is_empty());
        let server = normalize_windows_component(parts.next()?)?;
        let share = normalize_windows_component(parts.next()?)?;
        let components = parts
            .map(normalize_windows_component)
            .collect::<Option<Vec<_>>>()?;
        return Some(WindowsAbsolutePath {
            volume: format!("unc:{server}/{share}"),
            components,
        });
    }

    let bytes = normalized.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'/' {
        return None;
    }
    let components = normalized[3..]
        .split('/')
        .filter(|part| !part.is_empty())
        .map(normalize_windows_component)
        .collect::<Option<Vec<_>>>()?;
    Some(WindowsAbsolutePath {
        volume: format!("drive:{}", char::from(bytes[0]).to_ascii_lowercase()),
        components,
    })
}

#[cfg(any(target_os = "windows", test))]
fn normalize_windows_component(component: &str) -> Option<String> {
    if component == "." || component == ".." {
        return None;
    }
    Some(component.to_lowercase())
}

#[cfg(any(target_os = "windows", test))]
fn strip_prefix_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = value.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &value[prefix.len()..])
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn avx2_available() -> bool {
    std::arch::is_x86_feature_detected!("avx2")
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn avx2_available() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_reports_real_storage() {
        let dir = tempfile::tempdir().unwrap();
        let report = diagnose_hardware(dir.path()).unwrap();
        assert!(report.available_disk_bytes > 0);
        assert!(report.total_memory_bytes > 0);
    }

    #[test]
    fn windows_mount_matching_normalizes_verbatim_prefix_separators_and_case() {
        assert!(windows_path_is_on_mount(
            r"\\?\C:\Users\Michael\AirWiki\data",
            r"c:/users/MICHAEL",
        ));
        assert!(windows_path_is_on_mount(
            r"C:\Users\Michael\AirWiki\data",
            r"\\?\c:\users\michael\",
        ));
    }

    #[test]
    fn windows_mount_matching_rejects_other_volumes_and_partial_components() {
        assert!(!windows_path_is_on_mount(r"\\?\D:\AirWiki\data", r"C:\",));
        assert!(!windows_path_is_on_mount(
            r"C:\AirWiki-backup\data",
            r"C:\AirWiki",
        ));
        assert!(!windows_path_is_on_mount(r"C:relative", r"C:\"));
        assert!(!windows_path_is_on_mount(r"\\.\C:\data", r"\\.\C:\"));
    }

    #[test]
    fn windows_mount_matching_supports_normal_and_verbatim_unc_paths() {
        assert!(windows_path_is_on_mount(
            r"\\?\UNC\Server\Knowledge\AirWiki\data",
            r"\\server\knowledge",
        ));
        assert!(!windows_path_is_on_mount(
            r"\\server\knowledge-other\AirWiki",
            r"\\server\knowledge",
        ));
        assert!(!windows_path_is_on_mount(
            r"\\server-two\knowledge\AirWiki",
            r"\\server\knowledge",
        ));
    }
}
