//! Persisted per-install updater state: the runtime-selected release channel
//! and the random install id used for staged-rollout bucketing.
//!
//! Stored as JSON at `<data_dir>/<app>/updater-state.json` (`crate::app_dirs`).
//! Without a data directory the state still works in memory for the process
//! lifetime.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tynd_update_core::validate_channel;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateFile {
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    install_id: Option<String>,
}

pub(super) struct UpdaterState {
    path: Option<PathBuf>,
    data: StateFile,
}

static STATE: LazyLock<Mutex<UpdaterState>> =
    LazyLock::new(|| Mutex::new(UpdaterState::load(default_path())));

fn default_path() -> Option<PathBuf> {
    Some(crate::app_dirs::app_dir(crate::app_dirs::AppDir::Data)?.join("updater-state.json"))
}

/// Runtime channel override, if the app ever called `setChannel`.
pub(super) fn runtime_channel() -> Option<String> {
    STATE.lock().channel()
}

pub(super) fn set_runtime_channel(name: &str) -> Result<(), String> {
    STATE.lock().set_channel(name)
}

/// Stable random id of this install (generated and persisted on first use).
pub(super) fn install_id() -> String {
    STATE.lock().install_id()
}

impl UpdaterState {
    pub(super) fn load(path: Option<PathBuf>) -> Self {
        let data = path
            .as_deref()
            .and_then(|p| fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<StateFile>(&bytes).ok())
            .map(|mut data| {
                // Ignore a hand-edited / corrupt channel rather than using it in URLs.
                if data
                    .channel
                    .as_deref()
                    .is_some_and(|c| validate_channel(c).is_err())
                {
                    data.channel = None;
                }
                data
            })
            .unwrap_or_default();
        Self { path, data }
    }

    pub(super) fn channel(&self) -> Option<String> {
        self.data.channel.clone()
    }

    pub(super) fn set_channel(&mut self, name: &str) -> Result<(), String> {
        validate_channel(name).map_err(|e| format!("updater.setChannel: {e}"))?;
        self.data.channel = Some(name.to_string());
        self.save();
        Ok(())
    }

    pub(super) fn install_id(&mut self) -> String {
        if let Some(id) = &self.data.install_id {
            return id.clone();
        }
        let id = random_id();
        self.data.install_id = Some(id.clone());
        self.save();
        id
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Err(error) = write_atomic(path, &self.data) {
            tracing::warn!(
                target: "tynd::updater",
                error = %error,
                "updater: could not persist state"
            );
        }
    }
}

fn write_atomic(path: &Path, data: &StateFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(data).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

fn random_id() -> String {
    use std::fmt::Write as _;
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // Extremely unlikely; fall back to time + pid so the id is still unique per install.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        bytes.copy_from_slice(&(nanos ^ u128::from(std::process::id())).to_le_bytes());
    }
    bytes.iter().fold(String::with_capacity(32), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_and_install_id_persist_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("updater-state.json");

        let mut state = UpdaterState::load(Some(path.clone()));
        assert_eq!(state.channel(), None);
        state.set_channel("beta").unwrap();
        let id = state.install_id();
        assert_eq!(id.len(), 32);
        assert_eq!(state.install_id(), id, "id is stable within a process");

        let reloaded = UpdaterState::load(Some(path));
        assert_eq!(reloaded.channel().as_deref(), Some("beta"));
        let mut reloaded = reloaded;
        assert_eq!(reloaded.install_id(), id, "id survives a restart");
    }

    #[test]
    fn rejects_invalid_channel_and_ignores_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updater-state.json");
        let mut state = UpdaterState::load(Some(path.clone()));
        assert!(state.set_channel("../evil").is_err());
        assert_eq!(state.channel(), None);

        fs::write(&path, r#"{"channel":"a/b","installId":"x"}"#).unwrap();
        let loaded = UpdaterState::load(Some(path.clone()));
        assert_eq!(loaded.channel(), None);

        fs::write(&path, "not json").unwrap();
        assert_eq!(UpdaterState::load(Some(path)).channel(), None);
    }

    #[test]
    fn works_without_a_data_dir() {
        let mut state = UpdaterState::load(None);
        state.set_channel("nightly").unwrap();
        assert_eq!(state.channel().as_deref(), Some("nightly"));
        assert_ne!(state.install_id(), "");
    }
}
