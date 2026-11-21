use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

pub use error::{Error, Result};
pub use models::*;

#[cfg(desktop)]
mod desktop;
#[cfg(mobile)]
mod mobile;

mod error;
mod models;

#[cfg(desktop)]
use desktop::CarminiumAudio;
#[cfg(mobile)]
use mobile::CarminiumAudio;

pub trait CarminiumAudioExt<R: Runtime> {
    fn carminium_audio(&self) -> &CarminiumAudio<R>;
}

impl<R: Runtime, T: Manager<R>> CarminiumAudioExt<R> for T {
    fn carminium_audio(&self) -> &CarminiumAudio<R> {
        self.state::<CarminiumAudio<R>>().inner()
    }
}

#[tauri::command]
fn open_airplay<R: Runtime>(app: tauri::AppHandle<R>) -> Result<()> {
    app.carminium_audio().open_airplay()
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("carminium-audio")
        .invoke_handler(tauri::generate_handler![open_airplay])
        .setup(|app, api| {
            #[cfg(mobile)]
            let audio = mobile::init(app, api)?;
            #[cfg(desktop)]
            let audio = desktop::init(app, api)?;
            app.manage(audio);
            Ok(())
        })
        .build()
}
