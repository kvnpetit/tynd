use std::{collections::BTreeMap, num::NonZeroU64};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub identifier: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub remote: Option<RemoteCapability>,
    #[serde(default = "default_true")]
    pub local: bool,
    #[serde(default)]
    pub windows: Vec<String>,
    #[serde(default)]
    pub webviews: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<PermissionEntry>,
    #[serde(default)]
    pub platforms: Option<Vec<Target>>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteCapability {
    #[serde(default)]
    pub urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PermissionEntry {
    PermissionRef(String),
    Inline(Permission),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default)]
    pub default_permission: Option<String>,
    #[serde(default)]
    pub permissions: BTreeMap<String, Permission>,
    #[serde(default)]
    pub permission_sets: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub global_scope_schema: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Permission {
    #[serde(default)]
    pub version: Option<NonZeroU64>,
    pub identifier: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub commands: Commands,
    #[serde(default)]
    pub scope: Value,
    #[serde(default)]
    pub platforms: Option<Vec<Target>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Commands {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Target {
    #[serde(rename = "linux")]
    Linux,
    #[serde(rename = "macOS")]
    MacOS,
    #[serde(rename = "windows")]
    Windows,
    #[serde(rename = "android")]
    Android,
    #[serde(rename = "iOS")]
    Ios,
}

impl Target {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOS
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "android") {
            Self::Android
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else {
            Self::Linux
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum CapabilityFile {
    Capability(Capability),
    List(Vec<Capability>),
    NamedList { capabilities: Vec<Capability> },
}

impl CapabilityFile {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }

    pub fn into_capabilities(self) -> Vec<Capability> {
        match self {
            Self::Capability(c) => vec![c],
            Self::List(list) | Self::NamedList { capabilities: list } => list,
        }
    }
}
