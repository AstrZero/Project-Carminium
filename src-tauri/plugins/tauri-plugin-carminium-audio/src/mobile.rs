use std::sync::Arc;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tauri::{
    ipc::Channel,
    plugin::{PluginApi, PluginHandle},
    AppHandle, Runtime,
};
use tokio::sync::mpsc;

use crate::models::*;

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_carminium_audio);

#[cfg(target_os = "android")]
const ANDROID_PLUGIN_IDENTIFIER: &str = "com.yunofactory.carminium.audio";

enum NativeCmd {
    SetEventChannel(Channel),
    Play(PlayRequest),
    Pause,
    Resume,
    Stop,
    Seek(f64),
    SetVolume(f32),
    SetRate(f32),
    UpdateMetadata(TrackMetadata),
    OpenAirPlay,
}

#[derive(Serialize)]
struct SetChannelPayload {
    channel: Channel,
}

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> crate::Result<CarminiumAudio<R>> {
    #[cfg(target_os = "ios")]
    let handle = api.register_ios_plugin(init_plugin_carminium_audio)?;
    #[cfg(target_os = "android")]
    let handle = api.register_android_plugin(ANDROID_PLUGIN_IDENTIFIER, "CarminiumAudioPlugin")?;
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    let handle = {
        let _ = api;
        return Err(crate::Error::Message(
            "tauri-plugin-carminium-audio mobile backend is only supported on iOS and Android"
                .into(),
        ));
    };

    let handle = Arc::new(handle);
    let (tx, mut rx) = mpsc::unbounded_channel::<NativeCmd>();
    let worker_handle = Arc::clone(&handle);

    // Strictly serialized async dispatcher: NEVER blocks the iOS Main Thread or
    // Tauri's IPC thread (which previously deadlocked against DispatchQueue.main.async
    // inside `setup()`), while preserving exact FIFO ordering of playback commands.
    tauri::async_runtime::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            let res: Result<serde_json::Value, tauri::plugin::mobile::PluginInvokeError> =
                match cmd {
                    NativeCmd::SetEventChannel(channel) => {
                        worker_handle
                            .run_mobile_plugin_async(
                                "setEventChannel",
                                SetChannelPayload { channel },
                            )
                            .await
                    }
                    NativeCmd::Play(payload) => {
                        worker_handle.run_mobile_plugin_async("play", payload).await
                    }
                    NativeCmd::Pause => {
                        worker_handle.run_mobile_plugin_async("pause", ()).await
                    }
                    NativeCmd::Resume => {
                        worker_handle.run_mobile_plugin_async("resume", ()).await
                    }
                    NativeCmd::Stop => {
                        worker_handle.run_mobile_plugin_async("stop", ()).await
                    }
                    NativeCmd::Seek(position_ms) => {
                        worker_handle
                            .run_mobile_plugin_async("seek", SeekRequest { position_ms })
                            .await
                    }
                    NativeCmd::SetVolume(level) => {
                        worker_handle
                            .run_mobile_plugin_async("setVolume", VolumeRequest { level })
                            .await
                    }
                    NativeCmd::SetRate(rate) => {
                        worker_handle
                            .run_mobile_plugin_async("setRate", RateRequest { rate })
                            .await
                    }
                    NativeCmd::UpdateMetadata(metadata) => {
                        worker_handle
                            .run_mobile_plugin_async("updateMetadata", metadata)
                            .await
                    }
                    NativeCmd::OpenAirPlay => {
                        worker_handle
                            .run_mobile_plugin_async("openAirPlay", ())
                            .await
                    }
                };
            if let Err(e) = res {
                log::warn!("[carminium-audio] mobile plugin command failed: {e}");
            }
        }
    });

    Ok(CarminiumAudio { handle, tx })
}

pub struct CarminiumAudio<R: Runtime> {
    handle: Arc<PluginHandle<R>>,
    tx: mpsc::UnboundedSender<NativeCmd>,
}

impl<R: Runtime> CarminiumAudio<R> {
    fn send_cmd(&self, cmd: NativeCmd) -> crate::Result<()> {
        self.tx
            .send(cmd)
            .map_err(|_| crate::Error::Message("audio command queue closed".into()))
    }

    pub fn set_event_channel(&self, channel: Channel) -> crate::Result<()> {
        self.send_cmd(NativeCmd::SetEventChannel(channel))
    }

    pub fn play(&self, payload: PlayRequest) -> crate::Result<()> {
        self.send_cmd(NativeCmd::Play(payload))
    }

    pub fn pause(&self) -> crate::Result<()> {
        self.send_cmd(NativeCmd::Pause)
    }

    pub fn resume(&self) -> crate::Result<()> {
        self.send_cmd(NativeCmd::Resume)
    }

    pub fn stop(&self) -> crate::Result<()> {
        self.send_cmd(NativeCmd::Stop)
    }

    pub fn seek(&self, position_ms: f64) -> crate::Result<()> {
        self.send_cmd(NativeCmd::Seek(position_ms))
    }

    pub fn set_volume(&self, level: f32) -> crate::Result<()> {
        self.send_cmd(NativeCmd::SetVolume(level))
    }

    pub fn set_rate(&self, rate: f32) -> crate::Result<()> {
        self.send_cmd(NativeCmd::SetRate(rate))
    }

    pub fn update_metadata(&self, metadata: TrackMetadata) -> crate::Result<()> {
        self.send_cmd(NativeCmd::UpdateMetadata(metadata))
    }

    pub fn open_airplay(&self) -> crate::Result<()> {
        self.send_cmd(NativeCmd::OpenAirPlay)
    }

    pub async fn pick_audio_files(
        &self,
        destination_dir: String,
    ) -> crate::Result<PickFilesResponse> {
        self.handle
            .run_mobile_plugin_async(
                "pickAudioFiles",
                PickFilesRequest { destination_dir },
            )
            .await
            .map_err(Into::into)
    }

    pub async fn http_fetch(
        &self,
        payload: HttpFetchRequest,
    ) -> crate::Result<HttpFetchResponse> {
        self.handle
            .run_mobile_plugin_async("httpFetch", payload)
            .await
            .map_err(Into::into)
    }

    pub async fn get_diagnostics(&self) -> crate::Result<AudioDiagnosticsResponse> {
        self.handle
            .run_mobile_plugin_async("getDiagnostics", ())
            .await
            .map_err(Into::into)
    }

    pub async fn scan_device_music(&self) -> crate::Result<ScanDeviceMusicResponse> {
        #[cfg(target_os = "android")]
        {
            self.handle
                .run_mobile_plugin_async("scanDeviceMusic", ())
                .await
                .map_err(Into::into)
        }
        #[cfg(not(target_os = "android"))]
        {
            Ok(ScanDeviceMusicResponse {
                music_dirs: Vec::new(),
                permission_granted: false,
            })
        }
    }
}
