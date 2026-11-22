use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackMetadata {
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Optional local file path to a cached JPEG/PNG cover image for MPNowPlayingInfoCenter.
    pub artwork_path: Option<String>,
    pub duration_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayRequest {
    /// Local `file://...` URL or remote HTTP/HTTPS audio stream URL.
    pub url: String,
    /// Optional HTTP request headers (e.g. `Authorization: Basic ...` for WebDAV).
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub metadata: TrackMetadata,
    /// Optional initial seek offset in milliseconds.
    pub start_position_ms: Option<f64>,
    pub auto_play: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeekRequest {
    pub position_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeRequest {
    /// 0.0 ..= 1.0
    pub level: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateRequest {
    pub rate: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickFilesRequest {
    /// Destination directory where picked audio files will be copied (inside App Documents).
    pub destination_dir: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickFilesResponse {
    pub copied_paths: Vec<String>,
    pub imported_dir: Option<String>,
    pub canceled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpFetchRequest {
    pub url: String,
    pub method: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub timeout_secs: Option<f64>,
    #[serde(default)]
    pub response_base64: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpFetchResponse {
    pub status: u16,
    pub body: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDiagnosticsResponse {
    pub text: String,
}

/// Native playback & system remote events streamed from Swift AVPlayer via Tauri Channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NativeAudioEvent {
    #[serde(rename_all = "camelCase")]
    Tick {
        position_ms: f64,
        duration_ms: f64,
        status: String,
        rate: f64,
        track_id: String,
    },
    #[serde(rename_all = "camelCase")]
    State {
        status: String,
        track_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Ended {
        track_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Remote {
        action: String,
        position_ms: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    Error {
        message: String,
        track_id: String,
    },
}
