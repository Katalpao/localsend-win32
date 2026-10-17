//! Wire types for the LocalSend v2.2 REST protocol plus the in-memory
//! `DeviceInfo` record we keep per discovered peer.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_version() -> String {
    "2.0".to_string()
}

/// JSON body exchanged during multicast discovery and the `/register` endpoint.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Announce {
    pub alias: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    pub fingerprint: String,
    pub port: u16,
    /// "http" | "https"
    pub protocol: String,
    #[serde(default)]
    pub download: bool,
    #[serde(default)]
    pub announce: bool,
}

/// Our local record of a peer. `ip` / `announce` are not part of the wire format.
#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub alias: String,
    pub version: String,
    pub device_model: Option<String>,
    pub device_type: Option<String>,
    pub fingerprint: String,
    pub port: u16,
    pub protocol: String,
    pub download: bool,
    /// LAN IP we observed the peer on.
    pub ip: String,
}

impl DeviceInfo {
    pub fn from_announce(a: &Announce, ip: String) -> Self {
        DeviceInfo {
            alias: a.alias.clone(),
            version: a.version.clone(),
            device_model: a.device_model.clone(),
            device_type: a.device_type.clone(),
            fingerprint: a.fingerprint.clone(),
            port: a.port,
            protocol: a.protocol.clone(),
            download: a.download,
            ip,
        }
    }

    /// Stable identity used as a map key.
    pub fn id(&self) -> String {
        format!("{}:{}", self.ip, self.port)
    }

    /// Base URL for talking to this peer.
    pub fn base_url(&self) -> String {
        format!("{}://{}:{}", self.protocol, self.ip, self.port)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    #[serde(default)]
    pub modified: Option<String>,
    #[serde(default)]
    pub accessed: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileMeta {
    pub id: String,
    pub file_name: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<FileMetadata>,
}

/// `POST /api/localsend/v2/prepare-upload`
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadRequest {
    pub info: Announce,
    pub files: HashMap<String, FileMeta>,
}

/// Response of `prepare-upload`.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PrepareUploadResponse {
    pub session_id: String,
    pub files: HashMap<String, String>, // fileId -> token
}

/// `GET /api/localsend/v2/info`
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InfoResponse {
    pub alias: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
    pub fingerprint: String,
    pub download: bool,
}
