use crate::cover;
use crate::db::{LibraryDb, Track};
use crate::player::{emit_bridge, PlayerState};
use crate::settings::AppSettings;
use crate::{lyrics, scanner, subsonic, webdav};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_carminium_audio::CarminiumAudioExt;

pub struct AppPaths {
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub doc_dir: PathBuf,
    pub music_dir: PathBuf,
    pub private_libraries_dir: PathBuf,
}

fn dump(v: &impl serde::Serialize) -> Value {
    Value::String(serde_json::to_string(v).unwrap_or_else(|_| "null".to_string()))
}

fn emit_folders_updated<R: Runtime>(app: &AppHandle<R>, db: &LibraryDb) {
    emit_bridge(app, "folders_updated", dump(&db.get_folders().unwrap_or_default()));
}

fn emit_playlists_changed<R: Runtime>(app: &AppHandle<R>, db: &LibraryDb) {
    emit_bridge(app, "playlists_changed", dump(&db.get_playlists().unwrap_or_default()));
}

fn emit_liked_tracks_changed<R: Runtime>(app: &AppHandle<R>, db: &LibraryDb) {
    emit_bridge(app, "liked_tracks_changed", dump(&db.get_liked_tracks().unwrap_or_default()));
}

fn emit_remote_servers_changed<R: Runtime>(app: &AppHandle<R>, db: &LibraryDb) {
    let sub = db.get_subsonic_servers().unwrap_or_default();
    emit_bridge(app, "subsonic_servers_changed", dump(&sub));
    let mut all = sub;
    all.extend(db.get_webdav_servers().unwrap_or_default());
    emit_bridge(app, "remote_servers_changed", dump(&all));
}

fn arg_str(args: &[Value], idx: usize) -> String {
    args.get(idx)
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => String::new(),
        })
        .unwrap_or_default()
}

fn arg_i64(args: &[Value], idx: usize, default: i64) -> i64 {
    args.get(idx)
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_f64().map(|f| f as i64))
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .unwrap_or(default)
}

fn arg_f64(args: &[Value], idx: usize, default: f64) -> f64 {
    args.get(idx)
        .and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
        })
        .unwrap_or(default)
}

fn arg_bool(args: &[Value], idx: usize, default: bool) -> bool {
    args.get(idx)
        .and_then(|v| {
            v.as_bool().or_else(|| match v.as_str() {
                Some("true") | Some("1") => Some(true),
                Some("false") | Some("0") => Some(false),
                _ => None,
            })
        })
        .unwrap_or(default)
}

#[tauri::command]
pub fn register_playback_channel<R: Runtime>(
    app: AppHandle<R>,
    channel: Channel<Value>,
) -> Result<(), String> {
    let player = app.state::<Arc<PlayerState>>();
    player.set_frontend_channel(channel);
    Ok(())
}

#[tauri::command]
pub async fn dispatch_bridge<R: Runtime>(
    app: AppHandle<R>,
    method: String,
    args: Vec<Value>,
) -> Result<Value, String> {
    let db = app.state::<Arc<LibraryDb>>().inner().clone();
    let settings = app.state::<Arc<AppSettings>>().inner().clone();
    let player = app.state::<Arc<PlayerState>>().inner().clone();
    let paths = app.state::<Arc<AppPaths>>().inner().clone();

    let sep = settings.get_str("artist_separators", ";");
    let smart_albums = settings.get_bool("album_smart_grouping", true);

    let result: Value = match method.as_str() {
        // ── Library & Browsing ──────────────────────────────────────
        "get_library" => {
            let tracks = db.get_all_tracks_json(&sep).unwrap_or_default();
            dump(&tracks)
        }
        "get_albums" => {
            let albums = db.get_albums(&sep, smart_albums).unwrap_or_default();
            dump(&albums)
        }
        "get_artists" => {
            let artists = db.get_artists(&sep).unwrap_or_default();
            dump(&artists)
        }
        "get_folders" => {
            let folders = db.get_folders().unwrap_or_default();
            dump(&folders)
        }
        "get_album_tracks" => {
            let raw = arg_str(&args, 0);
            let parsed: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
            let album = parsed["album"].as_str().unwrap_or("");
            let aa = parsed["album_artist"].as_str();
            let album_keys: Option<Vec<(String, String)>> = parsed["album_keys"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|k| {
                            let ka = k["album"].as_str()?.to_string();
                            let kaa = k["album_artist"].as_str().unwrap_or("").to_string();
                            Some((ka, kaa))
                        })
                        .collect()
                });
            let tracks = db
                .get_album_tracks(album, aa, album_keys.as_deref())
                .unwrap_or_default();
            dump(&tracks)
        }
        "get_artist_tracks" => {
            let artist = arg_str(&args, 0);
            let tracks = db.get_artist_tracks(&artist, &sep).unwrap_or_default();
            dump(&tracks)
        }
        "search_tracks" => {
            let query = arg_str(&args, 0);
            let tracks = db.search_tracks(&query).unwrap_or_default();
            dump(&tracks)
        }

        // ── Local Files / iOS Document Picker ───────────────────────
        // Imported libraries are placed inside `private_libraries_dir` so they stay
        // separated from the user's public Files.app `Documents/` folder.
        "open_folder_dialog" => {
            let dest = paths.private_libraries_dir.to_string_lossy().to_string();
            match app.carminium_audio().pick_audio_files(dest).await {
                Ok(res) if !res.canceled && !res.copied_paths.is_empty() => {
                    let target = res
                        .imported_dir
                        .unwrap_or_else(|| paths.private_libraries_dir.to_string_lossy().to_string());
                    Value::String(target)
                }
                _ => Value::String(String::new()),
            }
        }
        "pick_audio_files" => {
            let dest = paths.private_libraries_dir.to_string_lossy().to_string();
            match app.carminium_audio().pick_audio_files(dest).await {
                Ok(res) if !res.canceled && !res.copied_paths.is_empty() => {
                    let target_str = res
                        .imported_dir
                        .unwrap_or_else(|| paths.private_libraries_dir.to_string_lossy().to_string());
                    let target_path = PathBuf::from(&target_str);
                    let _ = db.ensure_folder(&target_str);
                    let db_c = db.clone();
                    let app_c = app.clone();
                    tauri::async_runtime::spawn_blocking(move || {
                        let _ = scanner::scan_local_folder(&db_c, &target_path);
                        emit_folders_updated(&app_c, &db_c);
                        emit_bridge(&app_c, "library_updated", json!(""));
                    });
                    Value::String(target_str)
                }
                _ => Value::String(String::new()),
            }
        }
        "add_folder" => {
            let raw_path = arg_str(&args, 0);
            let target = if raw_path.is_empty() {
                paths.doc_dir.clone()
            } else {
                PathBuf::from(&raw_path)
            };
            let target_str = target.to_string_lossy().to_string();
            let _ = db.ensure_folder(&target_str);
            let db_c = db.clone();
            let app_c = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = scanner::scan_local_folder(&db_c, &target);
                emit_folders_updated(&app_c, &db_c);
                emit_bridge(&app_c, "library_updated", json!(""));
            });
            dump(&json!({ "path": target_str }))
        }
        "remove_folder" => {
            let folder_path = arg_str(&args, 0);
            let _ = db.remove_folder(&folder_path);
            // If this is a private imported library folder, clean up copied files on disk;
            // never delete user's public Files.app Documents/ directory.
            let p = PathBuf::from(&folder_path);
            if p.starts_with(&paths.private_libraries_dir) && p != paths.private_libraries_dir {
                let _ = std::fs::remove_dir_all(&p);
            }
            emit_folders_updated(&app, &db);
            emit_bridge(&app, "library_updated", json!(""));
            Value::Null
        }
        "rescan_folder" => {
            let raw_path = arg_str(&args, 0);
            let target = if raw_path.is_empty() {
                paths.doc_dir.clone()
            } else {
                PathBuf::from(&raw_path)
            };
            let target_str = target.to_string_lossy().to_string();
            let db_c = db.clone();
            let app_c = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let _ = scanner::scan_local_folder(&db_c, &target);
                emit_folders_updated(&app_c, &db_c);
                emit_bridge(&app_c, "library_updated", json!(""));
            });
            dump(&json!({ "path": target_str }))
        }

        // ── Playback & Queue Controls ───────────────────────────────
        "play_track" => {
            let tid = arg_str(&args, 0);
            if let Ok(Some(t)) = db.get_track(&tid) {
                player.play_tracks(&app, &db, vec![t], 0);
            }
            Value::Null
        }
        "play_from_list" => {
            let raw = arg_str(&args, 0);
            let index = arg_i64(&args, 1, 0).max(0) as usize;
            if let Ok(tracks) = serde_json::from_str::<Vec<Track>>(&raw) {
                player.play_tracks(&app, &db, tracks, index);
            }
            Value::Null
        }
        "play" => {
            player.play(&app, &db);
            Value::Null
        }
        "pause" => {
            player.pause(&app);
            Value::Null
        }
        "next_track" | "advance_to_next" => {
            player.next_track(&app, &db, false);
            Value::Null
        }
        "prev_track" => {
            player.prev_track(&app, &db);
            Value::Null
        }
        "seek" => {
            let pos_ms = arg_f64(&args, 0, 0.0);
            player.seek(&app, pos_ms);
            Value::Null
        }
        "set_volume" => {
            let level = arg_i64(&args, 0, 100);
            settings.set("volume", json!(level));
            player.set_volume(&app, level);
            Value::Null
        }
        "set_shuffle" => {
            let enabled = arg_bool(&args, 0, false);
            settings.set("shuffle", json!(enabled));
            player.set_shuffle(&app, enabled);
            Value::Null
        }
        "set_repeat" => {
            let mode = arg_str(&args, 0);
            settings.set("repeat", json!(&mode));
            player.set_repeat(&app, &mode);
            Value::Null
        }
        "set_rate" | "set_tempo" => {
            let rate = arg_f64(&args, 0, 1.0);
            player.set_rate(&app, rate);
            Value::Null
        }
        "get_rate" | "get_tempo" | "get_pitch" => json!(1.0),
        "open_airplay" => {
            let _ = app.carminium_audio().open_airplay();
            Value::Bool(true)
        }
        "add_next" => {
            let raw = arg_str(&args, 0);
            if let Ok(track) = serde_json::from_str::<Track>(&raw) {
                player.add_next(&app, track);
            }
            Value::Null
        }
        "append_queue" => {
            let raw = arg_str(&args, 0);
            if let Ok(track) = serde_json::from_str::<Track>(&raw) {
                player.append_queue(&app, track);
            }
            Value::Null
        }
        "remove_from_queue" => {
            let idx = arg_i64(&args, 0, -1);
            if idx >= 0 {
                player.remove_from_queue(&app, idx as usize);
            }
            Value::Null
        }
        "reorder_queue" => {
            let from = arg_i64(&args, 0, -1);
            let to = arg_i64(&args, 1, -1);
            if from >= 0 && to >= 0 {
                player.reorder_queue(&app, from as usize, to as usize);
            }
            Value::Null
        }
        "play_queue_at" => {
            let idx = arg_i64(&args, 0, -1);
            if idx >= 0 {
                player.play_queue_at(&app, &db, idx as usize);
            }
            Value::Null
        }
        "peek_next_track" => {
            if let Some(t) = player.peek_next_track() {
                dump(&t)
            } else {
                Value::String("{}".to_string())
            }
        }
        "get_player_state" => dump(&player.snapshot()),
        "get_queue" => dump(&player.queue_snapshot()),

        // ── Likes ───────────────────────────────────────────────────
        "toggle_liked" => {
            if let Some(cur) = player.current_track() {
                let liked = db.toggle_liked_track(&cur.id).unwrap_or(false);
                emit_bridge(&app, "liked_changed", json!(liked));
                emit_liked_tracks_changed(&app, &db);
                Value::Bool(liked)
            } else {
                Value::Bool(false)
            }
        }
        "is_current_liked" => {
            let liked = player
                .current_track()
                .map(|t| db.is_track_liked(&t.id))
                .unwrap_or(false);
            Value::Bool(liked)
        }
        "toggle_liked_track" => {
            let tid = arg_str(&args, 0);
            let liked = db.toggle_liked_track(&tid).unwrap_or(false);
            if player
                .current_track()
                .map(|t| t.id == tid)
                .unwrap_or(false)
            {
                emit_bridge(&app, "liked_changed", json!(liked));
            }
            emit_liked_tracks_changed(&app, &db);
            Value::Bool(liked)
        }
        "is_track_liked" => Value::Bool(db.is_track_liked(&arg_str(&args, 0))),
        "get_liked_tracks" => dump(&db.get_liked_tracks().unwrap_or_default()),
        "toggle_liked_artist" => {
            let name = arg_str(&args, 0);
            let liked = db.toggle_liked_artist(&name).unwrap_or(false);
            emit_bridge(
                &app,
                "liked_artists_changed",
                dump(&db.get_liked_artists(&sep).unwrap_or_default()),
            );
            Value::Bool(liked)
        }
        "is_artist_liked" => Value::Bool(db.is_artist_liked(&arg_str(&args, 0))),
        "get_liked_artists" => dump(&db.get_liked_artists(&sep).unwrap_or_default()),
        "toggle_liked_album" => {
            let album = arg_str(&args, 0);
            let aa = arg_str(&args, 1);
            let liked = db.toggle_liked_album(&album, &aa).unwrap_or(false);
            emit_bridge(
                &app,
                "liked_albums_changed",
                dump(&db.get_liked_albums(&sep, smart_albums).unwrap_or_default()),
            );
            Value::Bool(liked)
        }
        "is_album_liked" => {
            Value::Bool(db.is_album_liked(&arg_str(&args, 0), &arg_str(&args, 1)))
        }
        "get_liked_albums" => dump(&db.get_liked_albums(&sep, smart_albums).unwrap_or_default()),

        // ── History & Discover ──────────────────────────────────────
        "get_play_history" => {
            let limit = arg_i64(&args, 0, 200);
            dump(&db.get_play_history(limit).unwrap_or_default())
        }
        "clear_play_history" => {
            let _ = db.clear_play_history();
            emit_bridge(&app, "history_changed", Value::String("[]".to_string()));
            Value::Null
        }
        "get_play_stats" => dump(&db.get_play_stats(&sep).unwrap_or_else(|_| json!({}))),
        "get_daily_mixes" => dump(&db.get_daily_mixes(&sep).unwrap_or_default()),
        "get_explore_tracks" => dump(&db.get_explore_tracks(30).unwrap_or_else(|_| json!({}))),
        "get_similar_recommendations" => dump(&Vec::<Value>::new()),

        // ── Playlists ───────────────────────────────────────────────
        "get_playlists" => dump(&db.get_playlists().unwrap_or_default()),
        "create_playlist" => {
            let name = arg_str(&args, 0);
            let pl = db
                .create_playlist(&name)
                .unwrap_or_else(|_| json!({ "error": "create failed" }));
            emit_playlists_changed(&app, &db);
            dump(&pl)
        }
        "rename_playlist" => {
            let pid = arg_i64(&args, 0, 0);
            let name = arg_str(&args, 1);
            let _ = db.rename_playlist(pid, &name);
            emit_playlists_changed(&app, &db);
            Value::Null
        }
        "delete_playlist" => {
            let pid = arg_i64(&args, 0, 0);
            let _ = db.delete_playlist(pid);
            emit_playlists_changed(&app, &db);
            Value::Null
        }
        "add_to_playlist" => {
            let pid = arg_i64(&args, 0, 0);
            let tid = arg_str(&args, 1);
            let _ = db.add_tracks_to_playlist(pid, &[tid]);
            emit_playlists_changed(&app, &db);
            Value::Null
        }
        "add_tracks_to_playlist" => {
            let pid = arg_i64(&args, 0, 0);
            let raw = arg_str(&args, 1);
            let ids: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            let added = db.add_tracks_to_playlist(pid, &ids).unwrap_or(0);
            emit_playlists_changed(&app, &db);
            json!(added)
        }
        "remove_from_playlist" => {
            let pid = arg_i64(&args, 0, 0);
            let tid = arg_str(&args, 1);
            let _ = db.remove_from_playlist(pid, &tid);
            emit_playlists_changed(&app, &db);
            Value::Null
        }
        "get_playlist_tracks" => {
            let pid = arg_i64(&args, 0, 0);
            dump(&db.get_playlist_tracks(pid).unwrap_or_default())
        }
        "get_all_playlist_track_ids" => {
            dump(&db.get_all_playlist_track_ids().unwrap_or_default())
        }

        // ── Remote Sources (Subsonic / Navidrome & WebDAV) ──────────
        "get_subsonic_servers" => dump(&db.get_subsonic_servers().unwrap_or_default()),
        "add_subsonic_server" => {
            let name = arg_str(&args, 0);
            let url = arg_str(&args, 1);
            let user = arg_str(&args, 2);
            let pass = arg_str(&args, 3);
            let mode = {
                let m = arg_str(&args, 4);
                if m.is_empty() {
                    "subsonic".to_string()
                } else {
                    m
                }
            };
            let id = db
                .add_subsonic_server(&name, &url, &user, &pass, &mode)
                .unwrap_or(0);
            emit_remote_servers_changed(&app, &db);
            let srv = db.get_subsonic_server(id).ok().flatten();
            dump(&srv)
        }
        "update_subsonic_server" => {
            let id = arg_i64(&args, 0, 0);
            let name = arg_str(&args, 1);
            let url = arg_str(&args, 2);
            let user = arg_str(&args, 3);
            let pass = arg_str(&args, 4);
            let mode = {
                let m = arg_str(&args, 5);
                if m.is_empty() {
                    "subsonic".to_string()
                } else {
                    m
                }
            };
            let _ = db.update_subsonic_server(id, &name, &url, &user, &pass, &mode);
            emit_remote_servers_changed(&app, &db);
            dump(&db.get_subsonic_server(id).ok().flatten())
        }
        "remove_subsonic_server" => {
            let id = arg_i64(&args, 0, 0);
            let _ = db.remove_subsonic_server(id);
            emit_remote_servers_changed(&app, &db);
            emit_bridge(&app, "library_updated", json!(""));
            Value::Null
        }
        "test_subsonic_server" => {
            let id = arg_i64(&args, 0, 0);
            match db.get_subsonic_server(id).ok().flatten() {
                Some(srv) => match subsonic::test_server(&srv).await {
                    Ok(info) => dump(&info),
                    Err(e) => dump(&json!({ "ok": false, "error": e })),
                },
                None => dump(&json!({ "ok": false, "error": "服务器不存在" })),
            }
        }
        "sync_subsonic_server" => {
            let id = arg_i64(&args, 0, 0);
            let db_c = db.clone();
            let app_c = app.clone();
            tauri::async_runtime::spawn(async move {
                match db_c.get_subsonic_server(id).ok().flatten() {
                    Some(srv) => match subsonic::sync_server(&db_c, &srv).await {
                        Ok(stats) => {
                            emit_remote_servers_changed(&app_c, &db_c);
                            emit_bridge(&app_c, "library_updated", json!(""));
                            emit_bridge(
                                &app_c,
                                "subsonic_sync_result",
                                dump(&json!({ "ok": true, "server_id": id, "stats": stats })),
                            );
                        }
                        Err(err) => {
                            emit_bridge(
                                &app_c,
                                "subsonic_sync_result",
                                dump(&json!({ "ok": false, "server_id": id, "error": err })),
                            );
                        }
                    },
                    None => {
                        emit_bridge(
                            &app_c,
                            "subsonic_sync_result",
                            dump(&json!({ "ok": false, "server_id": id, "error": "服务器不存在" })),
                        );
                    }
                }
            });
            dump(&json!({ "ok": true, "started": true, "message": "同步已开始" }))
        }
        "get_webdav_servers" => dump(&db.get_webdav_servers().unwrap_or_default()),
        "add_webdav_server" => {
            let name = arg_str(&args, 0);
            let url = arg_str(&args, 1);
            let user = arg_str(&args, 2);
            let pass = arg_str(&args, 3);
            let id = db.add_webdav_server(&name, &url, &user, &pass).unwrap_or(0);
            emit_remote_servers_changed(&app, &db);
            dump(&db.get_webdav_server(id).ok().flatten())
        }
        "update_webdav_server" => {
            let id = arg_i64(&args, 0, 0);
            let name = arg_str(&args, 1);
            let url = arg_str(&args, 2);
            let user = arg_str(&args, 3);
            let pass = arg_str(&args, 4);
            let _ = db.update_webdav_server(id, &name, &url, &user, &pass);
            emit_remote_servers_changed(&app, &db);
            dump(&db.get_webdav_server(id).ok().flatten())
        }
        "remove_webdav_server" => {
            let id = arg_i64(&args, 0, 0);
            let _ = db.remove_webdav_server(id);
            emit_remote_servers_changed(&app, &db);
            emit_bridge(&app, "library_updated", json!(""));
            Value::Null
        }
        "test_webdav_server" => {
            let id = arg_i64(&args, 0, 0);
            match db.get_webdav_server(id).ok().flatten() {
                Some(srv) => match webdav::test_server(&srv).await {
                    Ok(info) => dump(&info),
                    Err(e) => dump(&json!({ "ok": false, "error": e })),
                },
                None => dump(&json!({ "ok": false, "error": "服务器不存在" })),
            }
        }
        "sync_webdav_server" => {
            let id = arg_i64(&args, 0, 0);
            let db_c = db.clone();
            let app_c = app.clone();
            tauri::async_runtime::spawn(async move {
                match db_c.get_webdav_server(id).ok().flatten() {
                    Some(srv) => match webdav::sync_server(&db_c, &srv).await {
                        Ok(stats) => {
                            emit_remote_servers_changed(&app_c, &db_c);
                            emit_bridge(&app_c, "library_updated", json!(""));
                            emit_bridge(
                                &app_c,
                                "subsonic_sync_result",
                                dump(&json!({ "ok": true, "server_id": id, "stats": stats })),
                            );
                        }
                        Err(err) => {
                            emit_bridge(
                                &app_c,
                                "subsonic_sync_result",
                                dump(&json!({ "ok": false, "server_id": id, "error": err })),
                            );
                        }
                    },
                    None => {
                        emit_bridge(
                            &app_c,
                            "subsonic_sync_result",
                            dump(&json!({ "ok": false, "server_id": id, "error": "服务器不存在" })),
                        );
                    }
                }
            });
            dump(&json!({ "ok": true, "started": true, "message": "同步已开始" }))
        }
        "get_remote_servers" => {
            let mut all = db.get_subsonic_servers().unwrap_or_default();
            all.extend(db.get_webdav_servers().unwrap_or_default());
            dump(&all)
        }
        "get_smb_servers" => Value::String("[]".to_string()),

        // ── Lyrics ──────────────────────────────────────────────────
        "search_netease_lyrics" => {
            let q = arg_str(&args, 0);
            dump(&lyrics::search_lyrics(&app, &q, "ncm").await)
        }
        "fetch_netease_lyrics" => {
            let id = arg_str(&args, 0);
            dump(&lyrics::fetch_lyrics_by_id(&app, &id, "ncm").await)
        }
        "search_lyrics" => {
            let q = arg_str(&args, 0);
            let src = arg_str(&args, 1);
            dump(&lyrics::search_lyrics(&app, &q, &src).await)
        }
        "fetch_lyrics" => {
            let id = arg_str(&args, 0);
            let src = arg_str(&args, 1);
            dump(&lyrics::fetch_lyrics_by_id(&app, &id, &src).await)
        }
        "get_subsonic_lyrics" => {
            let tid = arg_str(&args, 0);
            if let Ok(Some(track)) = db.get_track(&tid) {
                if track.source == "subsonic" {
                    if let Some(sid) = track.server_id {
                        if let Ok(Some(srv)) = db.get_subsonic_server(sid) {
                            let lrc = subsonic::get_lyrics(
                                &srv,
                                track.subsonic_id.as_deref().unwrap_or(""),
                                &track.artist,
                                &track.title,
                            )
                            .await;
                            return Ok(dump(&json!({ "lyrics": lrc, "error": null })));
                        }
                    }
                }
            }
            dump(&json!({ "lyrics": null, "error": "未找到歌词" }))
        }
        "apply_lyrics" => {
            let tid = arg_str(&args, 0);
            let text = arg_str(&args, 1);
            let _ = db.update_lyrics(&tid, &text);
            player.update_track_lyrics(&app, &tid, &text);
            dump(&json!({ "success": true }))
        }
        "apply_lyrics_temporary" => {
            let tid = arg_str(&args, 0);
            let text = arg_str(&args, 1);
            player.update_track_lyrics(&app, &tid, &text);
            dump(&json!({ "success": true }))
        }
        "get_embedded_lyrics" => {
            let tid = arg_str(&args, 0);
            let lrc = db
                .get_track(&tid)
                .ok()
                .flatten()
                .and_then(|t| {
                    if t.source == "local" {
                        scanner::read_embedded_lyrics(Path::new(&t.path))
                    } else {
                        t.lyrics
                    }
                })
                .unwrap_or_default();
            Value::String(lrc)
        }

        // ── Cover Colors, Artist Overrides, URLs ────────────────────
        "get_cover_base_url" | "get_media_base_url" => {
            Value::String("carminium://localhost".to_string())
        }
        "store_cover_colors" => {
            let tid = arg_str(&args, 0);
            let colors = arg_str(&args, 1);
            let _ = db.store_cover_colors(&tid, &colors);
            dump(&json!({ "ok": true }))
        }
        "get_cover_colors" => {
            let raw = arg_str(&args, 0);
            let ids: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            dump(&db.get_cover_colors(&ids).unwrap_or_default())
        }
        "get_artist_override" => {
            let name = arg_str(&args, 0);
            dump(&db.get_artist_override(&name).unwrap_or_else(|_| json!({})))
        }
        "save_artist_override" => {
            let name = arg_str(&args, 0);
            let raw = arg_str(&args, 1);
            let patch: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
            let aliases = patch.get("aliases").cloned();
            let bio = patch
                .get("bio")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let ov = db
                .save_artist_override(&name, aliases, bio)
                .unwrap_or_else(|_| json!({}));
            dump(&json!({ "ok": true, "override": ov, "version": crate::db::now_secs() as i64 }))
        }

        // ── Settings & Cache & App Info ─────────────────────────────
        "get_settings" => dump(&settings.all()),
        "save_settings" => {
            let raw = arg_str(&args, 0);
            if let Ok(Value::Object(patch)) = serde_json::from_str::<Value>(&raw) {
                let affects_library = patch.contains_key("artist_separators")
                    || patch.contains_key("album_smart_grouping");
                settings.update(&patch);
                let all_json = dump(&settings.all());
                emit_bridge(&app, "settings_changed", all_json);
                if affects_library {
                    emit_bridge(&app, "library_updated", json!(""));
                }
            }
            Value::Null
        }
        "stream_cache_stats" => {
            let (files, bytes) = cover::cache_stats(&db.cover_cache_dir);
            dump(&json!({
                "files": files,
                "bytes": bytes,
                "limitBytes": 2048_u64 * 1024 * 1024,
                "dir": paths.cache_dir.to_string_lossy(),
            }))
        }
        "clear_stream_cache" => {
            let (removed, freed) = cover::clear_cover_cache(&db.cover_cache_dir);
            dump(&json!({ "ok": true, "removed": removed, "freedBytes": freed }))
        }
        "renderer_ready" => Value::Bool(true),
        "get_app_info" => dump(&json!({
            "name": "Project Carminium",
            "version": env!("CARGO_PKG_VERSION"),
            "build": "iOS-Tauri",
            "codename": "Marionette",
            "platform": "ios",
            "arch": std::env::consts::ARCH,
            "runtime": "Tauri v2",
            "dataDir": paths.data_dir.to_string_lossy(),
            "musicDir": paths.music_dir.to_string_lossy(),
        })),
        "get_diagnostic_info" => dump(&json!({
            "platform": "ios",
            "runtime": "tauri-v2",
            "version": env!("CARGO_PKG_VERSION"),
            "audioEngine": "AVPlayer (iOS Native)",
        })),
        "check_for_update" => dump(&json!({
            "hasUpdate": false,
            "currentVersion": env!("CARGO_PKG_VERSION"),
            "latestVersion": env!("CARGO_PKG_VERSION"),
        })),

        // ── Desktop-only safe no-op stubs ───────────────────────────
        "get_audio_devices" => Value::String("[]".to_string()),
        "set_wasapi_exclusive" => Value::String("shared".to_string()),
        "get_system_accent_color" => Value::String(String::new()),
        "get_automix" | "get_radical_transitions" => Value::Bool(false),
        "get_crossfade_duration" => json!(0),
        "find_video_for_track"
        | "get_track_profile"
        | "get_track_loudness"
        | "get_track_quality" => Value::Null,
        "prefetch_artist_images" | "prefetch_artist_aliases" => json!({ "queued": 0 }),
        _ => Value::Null,
    };

    Ok(result)
}
