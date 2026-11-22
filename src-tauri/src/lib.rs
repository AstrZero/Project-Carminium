#![recursion_limit = "256"]

mod bridge;
mod cover;
mod db;
mod lyrics;
mod player;
mod scanner;
mod settings;
mod subsonic;
mod webdav;

use bridge::AppPaths;
use db::LibraryDb;
use player::{emit_bridge, PlayerState};
use settings::AppSettings;
use std::fs;
use std::sync::Arc;
use tauri::http::{Response, StatusCode};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::Manager;
use tauri_plugin_carminium_audio::{CarminiumAudioExt, NativeAudioEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_carminium_audio::init())
        .register_asynchronous_uri_scheme_protocol("carminium", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let uri = request.uri().clone();
            tauri::async_runtime::spawn(async move {
                let path = uri.path();
                let query = uri.query().unwrap_or("");
                let size_param: Option<u32> = query.split('&').find_map(|pair| {
                    let (k, v) = pair.split_once('=')?;
                    if k == "size" {
                        v.parse::<u32>().ok()
                    } else {
                        None
                    }
                });

                let db = app.state::<Arc<LibraryDb>>().inner().clone();

                if let Some(raw_id) = path.strip_prefix("/cover/") {
                    let track_id = urlencoding::decode(raw_id)
                        .map(|c| c.into_owned())
                        .unwrap_or_else(|_| raw_id.to_string());
                    if let Some((bytes, mime)) =
                        cover::resolve_cover_bytes(&db, &track_id, size_param).await
                    {
                        if let Ok(resp) = Response::builder()
                            .status(StatusCode::OK)
                            .header("Content-Type", mime)
                            .header("Cache-Control", "public, max-age=86400")
                            .header("Access-Control-Allow-Origin", "*")
                            .body(bytes)
                        {
                            responder.respond(resp);
                            return;
                        }
                    }
                } else if let Some(raw_name) = path.strip_prefix("/artist-image/") {
                    let artist_name = urlencoding::decode(raw_name)
                        .map(|c| c.into_owned())
                        .unwrap_or_else(|_| raw_name.to_string());
                    if let Ok(ov) = db.get_artist_override(&artist_name) {
                        if let Some(img_path) = ov["image_path"].as_str() {
                            if !img_path.is_empty() {
                                if let Ok(bytes) = fs::read(img_path) {
                                    if let Ok(resp) = Response::builder()
                                        .status(StatusCode::OK)
                                        .header("Content-Type", "image/jpeg")
                                        .header("Access-Control-Allow-Origin", "*")
                                        .body(bytes)
                                    {
                                        responder.respond(resp);
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }

                let not_found = Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header("Access-Control-Allow-Origin", "*")
                    .body(Vec::new())
                    .unwrap();
                responder.respond(not_found);
            });
        })
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("carminium_data"));
            let cache_dir = app
                .path()
                .app_cache_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("carminium_cache"));
            // On iOS:
            // - `doc_dir` (`Documents/`) is the public Files.app-visible library folder
            //   ("我的 iPhone → Project Carminium"); creating `Music/` inside guarantees
            //   the folder appears in Files.app immediately on first launch.
            // - `private_libraries_dir` (`Application Support/.../libraries/`) stores
            //   additional imported libraries privately outside Files.app.
            let doc_dir = app
                .path()
                .document_dir()
                .unwrap_or_else(|_| data_dir.clone());
            let public_music_dir = doc_dir.join("Music");
            let private_libraries_dir = data_dir.join("libraries");

            let _ = fs::create_dir_all(&data_dir);
            let _ = fs::create_dir_all(&cache_dir);
            let _ = fs::create_dir_all(&public_music_dir);
            let _ = fs::create_dir_all(&private_libraries_dir);

            let db = Arc::new(
                LibraryDb::open(&data_dir, &cache_dir).unwrap_or_else(|e| {
                    log::error!("primary LibraryDb::open failed: {e}, falling back to temp dir");
                    let fb_data = std::env::temp_dir().join("carminium_data_fb");
                    let fb_cache = std::env::temp_dir().join("carminium_cache_fb");
                    LibraryDb::open(&fb_data, &fb_cache)
                        .expect("failed to initialize fallback Carminium SQLite library")
                }),
            );
            let app_settings = Arc::new(AppSettings::load(&data_dir));
            let player_state = Arc::new(PlayerState::new(&app_settings));
            let app_paths = Arc::new(AppPaths {
                data_dir,
                cache_dir,
                doc_dir: doc_dir.clone(),
                music_dir: public_music_dir,
                private_libraries_dir,
            });

            app.manage(db.clone());
            app.manage(app_settings);
            app.manage(player_state.clone());
            app.manage(app_paths);

            // Bind Swift AVPlayer native events -> Rust PlayerState via Tauri Channel.
            // `set_event_channel` dispatches asynchronously over a FIFO channel so it
            // NEVER blocks the iOS Main Thread during `setup()`.
            let app_handle = app.handle().clone();
            let db_for_events = db.clone();
            let player_for_events = player_state.clone();
            let native_channel = Channel::new(move |body: InvokeResponseBody| {
                if let InvokeResponseBody::Json(raw_json) = body {
                    if let Ok(ev) = serde_json::from_str::<NativeAudioEvent>(&raw_json) {
                        player_for_events.handle_native_event(&app_handle, &db_for_events, ev);
                    }
                }
                Ok(())
            });
            if let Err(e) = app.carminium_audio().set_event_channel(native_channel) {
                log::warn!("failed to bind native iOS audio channel: {e}");
            }

            // Scan Files.app-shared Documents directory in background after UI first paint
            let scan_app = app.handle().clone();
            let scan_db = db.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    let _ = scanner::scan_local_folder(&scan_db, &doc_dir);
                    let folders_json =
                        serde_json::to_string(&scan_db.get_folders().unwrap_or_default())
                            .unwrap_or_else(|_| "[]".to_string());
                    emit_bridge(&scan_app, "folders_updated", serde_json::json!(folders_json));
                    emit_bridge(&scan_app, "library_updated", serde_json::json!(""));
                })
                .await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bridge::dispatch_bridge,
            bridge::register_playback_channel,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Project Carminium");
}
