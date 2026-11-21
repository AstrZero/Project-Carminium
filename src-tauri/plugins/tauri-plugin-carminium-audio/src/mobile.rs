use serde::de::DeserializeOwned;
use serde::Serialize;
use tauri::{
    ipc::Channel,
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};

use crate::models::*;

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_carminium_audio);

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> crate::Result<CarminiumAudio<R>> {
    #[cfg(target_os = "ios")]
    let handle = api.register_ios_plugin(init_plugin_carminium_audio)?;
    #[cfg(not(target_os = "ios"))]
    let handle = {
        let _ = api;
        return Err(crate::Error::Message(
            "tauri-plugin-carminium-audio mobile backend is only supported on iOS".into(),
        ));
    };
    Ok(CarminiumAudio(handle))
}

#[derive(Serialize)]
struct SetChannelPayload {
    channel: Channel,
}

pub struct CarminiumAudio<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> CarminiumAudio<R> {
    pub fn set_event_channel(&self, channel: Channel) -> crate::Result<()> {
        self.0
            .run_mobile_plugin("setEventChannel", SetChannelPayload { channel })
            .map_err(Into::into)
    }

    pub fn play(&self, payload: PlayRequest) -> crate::Result<()> {
        self.0.run_mobile_plugin("play", payload).map_err(Into::into)
    }

    pub fn pause(&self) -> crate::Result<()> {
        self.0.run_mobile_plugin("pause", ()).map_err(Into::into)
    }

    pub fn resume(&self) -> crate::Result<()> {
        self.0.run_mobile_plugin("resume", ()).map_err(Into::into)
    }

    pub fn stop(&self) -> crate::Result<()> {
        self.0.run_mobile_plugin("stop", ()).map_err(Into::into)
    }

    pub fn seek(&self, position_ms: f64) -> crate::Result<()> {
        self.0
            .run_mobile_plugin("seek", SeekRequest { position_ms })
            .map_err(Into::into)
    }

    pub fn set_volume(&self, level: f32) -> crate::Result<()> {
        self.0
            .run_mobile_plugin("setVolume", VolumeRequest { level })
            .map_err(Into::into)
    }

    pub fn set_rate(&self, rate: f32) -> crate::Result<()> {
        self.0
            .run_mobile_plugin("setRate", RateRequest { rate })
            .map_err(Into::into)
    }

    pub fn update_metadata(&self, metadata: TrackMetadata) -> crate::Result<()> {
        self.0
            .run_mobile_plugin("updateMetadata", metadata)
            .map_err(Into::into)
    }

    pub fn open_airplay(&self) -> crate::Result<()> {
        self.0
            .run_mobile_plugin("openAirPlay", ())
            .map_err(Into::into)
    }

    pub async fn pick_audio_files(
        &self,
        destination_dir: String,
    ) -> crate::Result<PickFilesResponse> {
        self.0
            .run_mobile_plugin_async(
                "pickAudioFiles",
                PickFilesRequest { destination_dir },
            )
            .await
            .map_err(Into::into)
    }
}
