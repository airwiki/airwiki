//! Per-user single-instance guard and a deliberately tiny activation channel.

use std::{
    ffi::OsString,
    io::{Read, Write},
    path::Path,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
#[cfg(not(target_os = "windows"))]
use interprocess::local_socket::GenericFilePath;
#[cfg(target_os = "windows")]
use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::{ListenerOptions, Stream, prelude::*};
use single_instance::SingleInstance;

use crate::paths::AppPaths;

const SHOW_REQUEST: &[u8] = b"SHOW\n";
const OK_RESPONSE: &[u8] = b"OK\n";
const MAX_ACTIVATION_BYTES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchMode {
    Foreground,
    Background,
}

impl LaunchMode {
    pub(crate) fn from_args(
        arguments: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, LaunchArgumentError> {
        let mut mode = Self::Foreground;
        for argument in arguments.into_iter().skip(1) {
            if argument == "--background" {
                mode = Self::Background;
            } else if argument.to_string_lossy().starts_with("-psn_") {
                // LaunchServices used this private process serial argument on
                // older macOS releases. It does not alter product behavior.
            } else {
                return Err(LaunchArgumentError::Unknown(argument));
            }
        }
        Ok(mode)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum LaunchArgumentError {
    #[error("unknown launch argument {0:?}")]
    Unknown(OsString),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivationAction {
    Show,
}

pub(crate) enum InstanceDisposition {
    Primary(PrimaryInstance),
    Secondary,
}

pub(crate) struct PrimaryInstance {
    _guard: SingleInstance,
    actions: Receiver<ActivationAction>,
}

impl PrimaryInstance {
    pub(crate) fn try_actions(&self) -> mpsc::TryIter<'_, ActivationAction> {
        self.actions.try_iter()
    }
}

pub(crate) fn prepare_instance(
    paths: &AppPaths,
    launch_mode: LaunchMode,
) -> Result<InstanceDisposition> {
    let key = activation_key(&paths.data);
    let lock_name = instance_lock_name(&paths.data, &key);
    let guard = SingleInstance::new(&lock_name).context("could not create instance guard")?;
    if !guard.is_single() {
        if launch_mode == LaunchMode::Foreground {
            send_show_with_retry(&paths.data, &key);
        }
        return Ok(InstanceDisposition::Secondary);
    }

    let (actions_tx, actions_rx) = mpsc::channel();
    start_activation_listener(&paths.data, &key, actions_tx)?;
    Ok(InstanceDisposition::Primary(PrimaryInstance {
        _guard: guard,
        actions: actions_rx,
    }))
}

fn start_activation_listener(
    root: &Path,
    key: &str,
    sender: Sender<ActivationAction>,
) -> Result<()> {
    let name = activation_name(root, key)?;
    let listener = ListenerOptions::new()
        .name(name)
        .try_overwrite(true)
        .create_sync()
        .context("could not bind activation channel")?;
    thread::Builder::new()
        .name("airwiki-activation".to_owned())
        .spawn(move || {
            for connection in listener.incoming() {
                let Ok(mut connection) = connection else {
                    continue;
                };
                let mut bytes = [0_u8; MAX_ACTIVATION_BYTES];
                let Ok(length) = connection.read(&mut bytes) else {
                    continue;
                };
                if decode_request(&bytes[..length]) == Some(ActivationAction::Show) {
                    let _ = sender.send(ActivationAction::Show);
                    let _ = connection.write_all(OK_RESPONSE);
                }
            }
        })
        .context("could not start activation listener")?;
    Ok(())
}

fn send_show_with_retry(root: &Path, key: &str) {
    for _ in 0..20 {
        if send_show(root, key).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn send_show(root: &Path, key: &str) -> Result<()> {
    let name = activation_name(root, key)?;
    let mut connection = Stream::connect(name).context("primary instance is not ready")?;
    connection
        .write_all(SHOW_REQUEST)
        .context("could not request activation")?;
    let mut response = [0_u8; OK_RESPONSE.len()];
    connection
        .read_exact(&mut response)
        .context("primary instance did not acknowledge activation")?;
    if response != OK_RESPONSE {
        bail!("primary instance returned an invalid activation acknowledgement");
    }
    Ok(())
}

fn decode_request(bytes: &[u8]) -> Option<ActivationAction> {
    (bytes == SHOW_REQUEST).then_some(ActivationAction::Show)
}

fn activation_key(root: &Path) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in root.as_os_str().to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("airwiki-{hash:016x}")
}

#[cfg(target_os = "windows")]
fn instance_lock_name(_root: &Path, key: &str) -> String {
    format!(r"Local\{key}")
}

#[cfg(not(target_os = "windows"))]
fn instance_lock_name(root: &Path, _key: &str) -> String {
    root.join("airwiki.lock").to_string_lossy().into_owned()
}

#[cfg(target_os = "windows")]
fn activation_name(_root: &Path, key: &str) -> Result<interprocess::local_socket::Name<'static>> {
    key.to_owned()
        .to_ns_name::<GenericNamespaced>()
        .context("invalid activation pipe name")
}

#[cfg(not(target_os = "windows"))]
fn activation_name(root: &Path, _key: &str) -> Result<interprocess::local_socket::Name<'static>> {
    root.join("activation.sock")
        .to_fs_name::<GenericFilePath>()
        .map(interprocess::local_socket::Name::into_owned)
        .context("invalid activation socket path")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreground_is_the_default_launch_mode() {
        let mode = LaunchMode::from_args([OsString::from("airwiki")]).unwrap();

        assert_eq!(mode, LaunchMode::Foreground);
    }

    #[test]
    fn background_argument_selects_hidden_launch() {
        let mode =
            LaunchMode::from_args([OsString::from("airwiki"), OsString::from("--background")])
                .unwrap();

        assert_eq!(mode, LaunchMode::Background);
    }

    #[test]
    fn unknown_launch_argument_is_rejected() {
        let error =
            LaunchMode::from_args([OsString::from("airwiki"), OsString::from("--endpoint")])
                .unwrap_err();

        assert!(matches!(error, LaunchArgumentError::Unknown(_)));
    }

    #[test]
    fn activation_protocol_accepts_only_show() {
        assert_eq!(decode_request(SHOW_REQUEST), Some(ActivationAction::Show));
    }

    #[test]
    fn activation_protocol_rejects_extra_bytes() {
        assert_eq!(decode_request(b"SHOW private\n"), None);
    }

    #[test]
    fn activation_key_is_stable_and_does_not_expose_the_path() {
        let key = activation_key(Path::new("/Users/alice/private/airwiki"));

        assert_eq!(key, "airwiki-f188e7981fd11a8e");
    }
}
