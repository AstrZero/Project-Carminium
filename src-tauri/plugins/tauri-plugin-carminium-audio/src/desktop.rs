use serde::de::DeserializeOwned;
use tauri::{ipc::Channel, plugin::PluginApi, AppHandle, Runtime};

use crate::models::*;

pub fn init<R: Runtime, C: DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<CarminiumAudio<R>> {
    Ok(CarminiumAudio(app.clone()))
}

pub struct CarminiumAudio<R: Runtime>(#[allow(dead_code)] AppHandle<R>);

impl<R: Runtime> CarminiumAudio<R> {
    pub fn set_event_channel(&self, _channel: Channel) -> crate::Result<()> {
        Ok(())
    }

    pub fn play(&self, _payload: PlayRequest) -> crate::Result<()> {
        Ok(())
    }

    pub fn pause(&self) -> crate::Result<()> {
        Ok(())
    }

    pub fn resume(&self) -> crate::Result<()> {
        Ok(())
    }

    pub fn stop(&self) -> crate::Result<()> {
        Ok(())
    }

    pub fn seek(&self, _position_ms: f64) -> crate::Result<()> {
        Ok(())
    }

    pub fn set_volume(&self, _level: f32) -> crate::Result<()> {
        Ok(())
    }

    pub fn set_rate(&self, _rate: f32) -> crate::Result<()> {
        Ok(())
    }

    pub fn update_metadata(&self, _metadata: TrackMetadata) -> crate::Result<()> {
        Ok(())
    }

    pub fn open_airplay(&self) -> crate::Result<()> {
        Ok(())
    }

    pub async fn pick_audio_files(
        &self,
        _destination_dir: String,
    ) -> crate::Result<PickFilesResponse> {
        Ok(PickFilesResponse {
            copied_paths: Vec::new(),
            imported_dir: None,
            canceled: true,
        })
    }

    pub async fn http_fetch(
        &self,
        _payload: HttpFetchRequest,
    ) -> crate::Result<HttpFetchResponse> {
        Ok(HttpFetchResponse {
            status: 0,
            body: String::new(),
            error: Some("desktop_unavailable".to_string()),
        })
    }

    pub async fn get_diagnostics(&self) -> crate::Result<AudioDiagnosticsResponse> {
        Ok(AudioDiagnosticsResponse {
            text: "Diagnostics only available on mobile.".to_string(),
        })
    }

    pub async fn scan_device_music(&self) -> crate::Result<ScanDeviceMusicResponse> {
        Ok(ScanDeviceMusicResponse {
            music_dirs: Vec::new(),
            permission_granted: false,
        })
    }
}
