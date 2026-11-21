use crate::cover;
use crate::db::{LibraryDb, Track};
use crate::settings::AppSettings;
use crate::{subsonic, webdav};
use rand::seq::SliceRandom;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_carminium_audio::{
    CarminiumAudioExt, NativeAudioEvent, PlayRequest, TrackMetadata,
};

pub fn emit_bridge<R: Runtime>(app: &AppHandle<R>, event: &str, payload: Value) {
    let _ = app.emit(
        "bridge:event",
        json!({
            "event": event,
            "payload": payload,
        }),
    );
}

struct PlayerInner {
    original_queue: Vec<Track>,
    queue: Vec<Track>,
    current_index: isize,
    state: String,
    position_ms: f64,
    duration_ms: f64,
    volume: i64,
    shuffle: bool,
    repeat: String,
    rate: f64,
    frontend_channel: Option<Channel<Value>>,
}

pub struct PlayerState {
    inner: Mutex<PlayerInner>,
}

impl PlayerState {
    pub fn new(settings: &AppSettings) -> Self {
        let volume = settings.get_f64("volume", 100.0).clamp(0.0, 100.0) as i64;
        let shuffle = settings.get_bool("shuffle", false);
        let repeat = settings.get_str("repeat", "off");
        Self {
            inner: Mutex::new(PlayerInner {
                original_queue: Vec::new(),
                queue: Vec::new(),
                current_index: -1,
                state: "stopped".to_string(),
                position_ms: 0.0,
                duration_ms: 0.0,
                volume,
                shuffle,
                repeat,
                rate: 1.0,
                frontend_channel: None,
            }),
        }
    }

    pub fn set_frontend_channel(&self, channel: Channel<Value>) {
        let mut inner = self.inner.lock().unwrap();
        inner.frontend_channel = Some(channel);
    }

    pub fn snapshot(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        let current_track = if inner.current_index >= 0
            && (inner.current_index as usize) < inner.queue.len()
        {
            Some(inner.queue[inner.current_index as usize].clone())
        } else {
            None
        };
        json!({
            "current_track": current_track,
            "state": inner.state,
            "position": inner.position_ms,
            "duration": inner.duration_ms,
            "volume": inner.volume,
            "shuffle": inner.shuffle,
            "repeat": inner.repeat,
            "tempo": inner.rate,
            "pitch": 1.0,
            "rate": inner.rate,
        })
    }

    pub fn queue_snapshot(&self) -> Value {
        let inner = self.inner.lock().unwrap();
        let stripped: Vec<Value> = inner
            .queue
            .iter()
            .map(|t| {
                let mut v = serde_json::to_value(t).unwrap_or_else(|_| json!({}));
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("lyrics");
                }
                v
            })
            .collect();
        json!({
            "queue": stripped,
            "current_index": inner.current_index,
        })
    }

    pub fn current_track(&self) -> Option<Track> {
        let inner = self.inner.lock().unwrap();
        if inner.current_index >= 0 && (inner.current_index as usize) < inner.queue.len() {
            Some(inner.queue[inner.current_index as usize].clone())
        } else {
            None
        }
    }

    pub fn peek_next_track(&self) -> Option<Track> {
        let inner = self.inner.lock().unwrap();
        if inner.queue.is_empty() || inner.current_index < 0 {
            return None;
        }
        let next = (inner.current_index as usize) + 1;
        if next < inner.queue.len() {
            Some(inner.queue[next].clone())
        } else if inner.repeat == "all" {
            inner.queue.first().cloned()
        } else {
            None
        }
    }

    pub fn play_tracks<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        db: &Arc<LibraryDb>,
        mut tracks: Vec<Track>,
        start_index: usize,
    ) {
        if tracks.is_empty() {
            return;
        }
        // Hydrate tracks with DB metadata/lyrics if missing
        for t in tracks.iter_mut() {
            if t.lyrics.is_none() || t.path.is_empty() {
                if let Ok(Some(full)) = db.get_track(&t.id) {
                    *t = full;
                }
            }
        }
        let clamped_idx = start_index.min(tracks.len() - 1);
        let chosen = tracks[clamped_idx].clone();

        {
            let mut inner = self.inner.lock().unwrap();
            inner.original_queue = tracks.clone();
            if inner.shuffle && tracks.len() > 1 {
                let mut rest: Vec<Track> = tracks
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| *i != clamped_idx)
                    .map(|(_, t)| t)
                    .collect();
                let mut rng = rand::thread_rng();
                rest.shuffle(&mut rng);
                let mut shuffled = Vec::with_capacity(rest.len() + 1);
                shuffled.push(chosen);
                shuffled.extend(rest);
                inner.queue = shuffled;
                inner.current_index = 0;
            } else {
                inner.queue = tracks;
                inner.current_index = clamped_idx as isize;
            }
        }

        self.start_current_track(app, db);
    }

    pub fn play_queue_at<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        db: &Arc<LibraryDb>,
        index: usize,
    ) {
        {
            let mut inner = self.inner.lock().unwrap();
            if index >= inner.queue.len() {
                return;
            }
            inner.current_index = index as isize;
        }
        self.start_current_track(app, db);
    }

    pub fn play<R: Runtime>(&self, app: &AppHandle<R>, db: &Arc<LibraryDb>) {
        let should_start = {
            let inner = self.inner.lock().unwrap();
            inner.state == "stopped" && !inner.queue.is_empty()
        };
        if should_start {
            self.start_current_track(app, db);
            return;
        }
        let _ = app.carminium_audio().resume();
        let mut inner = self.inner.lock().unwrap();
        if inner.current_index >= 0 {
            inner.state = "playing".to_string();
        }
        drop(inner);
        emit_bridge(app, "playback_state_changed", json!("playing"));
    }

    pub fn pause<R: Runtime>(&self, app: &AppHandle<R>) {
        let _ = app.carminium_audio().pause();
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.state == "playing" {
                inner.state = "paused".to_string();
            }
        }
        emit_bridge(app, "playback_state_changed", json!("paused"));
    }

    pub fn seek<R: Runtime>(&self, app: &AppHandle<R>, position_ms: f64) {
        let clamped = position_ms.max(0.0);
        {
            let mut inner = self.inner.lock().unwrap();
            inner.position_ms = clamped;
        }
        let _ = app.carminium_audio().seek(clamped);
        emit_bridge(app, "position_changed", json!(clamped));
    }

    pub fn set_volume<R: Runtime>(&self, app: &AppHandle<R>, level: i64) {
        let clamped = level.clamp(0, 100);
        {
            let mut inner = self.inner.lock().unwrap();
            inner.volume = clamped;
        }
        let _ = app
            .carminium_audio()
            .set_volume((clamped as f32) / 100.0);
        emit_bridge(app, "volume_changed", json!(clamped));
    }

    pub fn set_rate<R: Runtime>(&self, app: &AppHandle<R>, rate: f64) {
        let clamped = rate.clamp(0.5, 2.0);
        {
            let mut inner = self.inner.lock().unwrap();
            inner.rate = clamped;
        }
        let _ = app.carminium_audio().set_rate(clamped as f32);
    }

    pub fn set_shuffle<R: Runtime>(&self, app: &AppHandle<R>, enabled: bool) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.shuffle = enabled;
            if !inner.queue.is_empty() && inner.current_index >= 0 {
                let current_id = inner.queue[inner.current_index as usize].id.clone();
                if enabled {
                    let cur = inner.queue[inner.current_index as usize].clone();
                    let mut rest: Vec<Track> = inner
                        .original_queue
                        .iter()
                        .filter(|t| t.id != current_id)
                        .cloned()
                        .collect();
                    let mut rng = rand::thread_rng();
                    rest.shuffle(&mut rng);
                    let mut new_q = Vec::with_capacity(rest.len() + 1);
                    new_q.push(cur);
                    new_q.extend(rest);
                    inner.queue = new_q;
                    inner.current_index = 0;
                } else {
                    inner.queue = inner.original_queue.clone();
                    inner.current_index = inner
                        .queue
                        .iter()
                        .position(|t| t.id == current_id)
                        .map(|i| i as isize)
                        .unwrap_or(0);
                }
            }
        }
        emit_bridge(app, "shuffle_changed", json!(enabled));
        self.emit_queue_changed(app);
    }

    pub fn set_repeat<R: Runtime>(&self, app: &AppHandle<R>, mode: &str) {
        let normalized = match mode {
            "all" => "all",
            "one" => "one",
            _ => "off",
        };
        {
            let mut inner = self.inner.lock().unwrap();
            inner.repeat = normalized.to_string();
        }
        emit_bridge(app, "repeat_changed", json!(normalized));
    }

    pub fn next_track<R: Runtime>(&self, app: &AppHandle<R>, db: &Arc<LibraryDb>, auto: bool) {
        let action = {
            let mut inner = self.inner.lock().unwrap();
            if inner.queue.is_empty() {
                return;
            }
            if auto && inner.repeat == "one" {
                Some(inner.current_index.max(0) as usize)
            } else {
                let next = inner.current_index + 1;
                if (next as usize) < inner.queue.len() {
                    inner.current_index = next;
                    Some(next as usize)
                } else if inner.repeat == "all" {
                    inner.current_index = 0;
                    Some(0)
                } else {
                    inner.state = "stopped".to_string();
                    inner.position_ms = 0.0;
                    None
                }
            }
        };

        if action.is_some() {
            self.start_current_track(app, db);
        } else {
            let _ = app.carminium_audio().stop();
            emit_bridge(app, "playback_state_changed", json!("stopped"));
            emit_bridge(app, "position_changed", json!(0));
        }
    }

    pub fn prev_track<R: Runtime>(&self, app: &AppHandle<R>, db: &Arc<LibraryDb>) {
        let restart_only = {
            let mut inner = self.inner.lock().unwrap();
            if inner.queue.is_empty() {
                return;
            }
            if inner.position_ms > 3500.0 {
                true
            } else if inner.current_index > 0 {
                inner.current_index -= 1;
                false
            } else if inner.repeat == "all" {
                inner.current_index = (inner.queue.len() - 1) as isize;
                false
            } else {
                true
            }
        };

        if restart_only {
            self.seek(app, 0.0);
        } else {
            self.start_current_track(app, db);
        }
    }

    pub fn add_next<R: Runtime>(&self, app: &AppHandle<R>, track: Track) {
        {
            let mut inner = self.inner.lock().unwrap();
            let pos = if inner.current_index < 0 {
                0
            } else {
                ((inner.current_index as usize) + 1).min(inner.queue.len())
            };
            inner.queue.insert(pos, track.clone());
            inner.original_queue.push(track);
        }
        self.emit_queue_changed(app);
    }

    pub fn append_queue<R: Runtime>(&self, app: &AppHandle<R>, track: Track) {
        {
            let mut inner = self.inner.lock().unwrap();
            inner.queue.push(track.clone());
            inner.original_queue.push(track);
        }
        self.emit_queue_changed(app);
    }

    pub fn remove_from_queue<R: Runtime>(&self, app: &AppHandle<R>, index: usize) {
        {
            let mut inner = self.inner.lock().unwrap();
            if index >= inner.queue.len() {
                return;
            }
            inner.queue.remove(index);
            if inner.queue.is_empty() {
                inner.current_index = -1;
            } else if (index as isize) < inner.current_index {
                inner.current_index -= 1;
            } else if (inner.current_index as usize) >= inner.queue.len() {
                inner.current_index = (inner.queue.len() - 1) as isize;
            }
        }
        self.emit_queue_changed(app);
    }

    pub fn reorder_queue<R: Runtime>(&self, app: &AppHandle<R>, from: usize, to: usize) {
        {
            let mut inner = self.inner.lock().unwrap();
            if from >= inner.queue.len() || to >= inner.queue.len() || from == to {
                return;
            }
            let current_id = if inner.current_index >= 0 {
                Some(inner.queue[inner.current_index as usize].id.clone())
            } else {
                None
            };
            let item = inner.queue.remove(from);
            inner.queue.insert(to, item);
            if let Some(cid) = current_id {
                if let Some(pos) = inner.queue.iter().position(|t| t.id == cid) {
                    inner.current_index = pos as isize;
                }
            }
        }
        self.emit_queue_changed(app);
    }

    pub fn update_track_lyrics<R: Runtime>(&self, app: &AppHandle<R>, track_id: &str, lyrics: &str) {
        let is_current = {
            let mut inner = self.inner.lock().unwrap();
            for t in inner.queue.iter_mut() {
                if t.id == track_id {
                    t.lyrics = Some(lyrics.to_string());
                }
            }
            for t in inner.original_queue.iter_mut() {
                if t.id == track_id {
                    t.lyrics = Some(lyrics.to_string());
                }
            }
            inner.current_index >= 0
                && inner
                    .queue
                    .get(inner.current_index as usize)
                    .map(|t| t.id == track_id)
                    .unwrap_or(false)
        };
        if is_current {
            emit_bridge(app, "lyrics_changed", json!(lyrics));
        }
    }

    pub fn emit_queue_changed<R: Runtime>(&self, app: &AppHandle<R>) {
        let q = self.queue_snapshot();
        emit_bridge(app, "queue_changed", json!(q.to_string()));
    }

    fn start_current_track<R: Runtime>(&self, app: &AppHandle<R>, db: &Arc<LibraryDb>) {
        let track = {
            let mut inner = self.inner.lock().unwrap();
            if inner.current_index < 0 || (inner.current_index as usize) >= inner.queue.len() {
                return;
            }
            let t = inner.queue[inner.current_index as usize].clone();
            inner.state = "playing".to_string();
            inner.position_ms = 0.0;
            inner.duration_ms = t.duration_ms as f64;
            t
        };

        let (url, headers) = resolve_playable_source(db, &track);
        let artwork_path = cover::ensure_artwork_path_sync(db, &track.id);
        let metadata = TrackMetadata {
            track_id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            artwork_path: artwork_path.clone(),
            duration_ms: if track.duration_ms > 0 {
                Some(track.duration_ms as f64)
            } else {
                None
            },
        };

        let _ = app.carminium_audio().play(PlayRequest {
            url,
            headers,
            metadata: metadata.clone(),
            start_position_ms: None,
            auto_play: Some(true),
        });

        // If Subsonic track has no local cached cover yet, fetch in background and refresh lockscreen
        if artwork_path.is_none() && track.source == "subsonic" {
            let app_clone = app.clone();
            let db_clone = Arc::clone(db);
            let mut meta_clone = metadata;
            let tid = track.id.clone();
            tauri::async_runtime::spawn(async move {
                if cover::resolve_cover_bytes(&db_clone, &tid, None)
                    .await
                    .is_some()
                {
                    let p = cover::cover_cache_path(&db_clone.cover_cache_dir, &tid);
                    if p.exists() {
                        meta_clone.artwork_path = Some(p.to_string_lossy().to_string());
                        let _ = app_clone.carminium_audio().update_metadata(meta_clone);
                    }
                }
            });
        }

        db.record_play(&track.id);
        let liked = db.is_track_liked(&track.id);
        let track_json = serde_json::to_string(&track).unwrap_or_else(|_| "{}".to_string());

        emit_bridge(app, "track_changed", json!(track_json));
        emit_bridge(app, "playback_state_changed", json!("playing"));
        emit_bridge(app, "duration_changed", json!(track.duration_ms));
        emit_bridge(app, "position_changed", json!(0));
        emit_bridge(app, "liked_changed", json!(liked));
        self.emit_queue_changed(app);
        let hist_json = serde_json::to_string(&db.get_play_history(200).unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        emit_bridge(app, "history_changed", json!(hist_json));
    }

    pub fn handle_native_event<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        db: &Arc<LibraryDb>,
        event: NativeAudioEvent,
    ) {
        match event {
            NativeAudioEvent::Tick {
                position_ms,
                duration_ms,
                status,
                rate,
                track_id,
            } => {
                let (dur_changed, channel_opt) = {
                    let mut inner = self.inner.lock().unwrap();
                    let prev_dur = inner.duration_ms;
                    inner.position_ms = position_ms;
                    if duration_ms > 0.0 {
                        inner.duration_ms = duration_ms;
                    }
                    let ch = inner.frontend_channel.clone();
                    ((inner.duration_ms - prev_dur).abs() > 500.0, ch)
                };
                if let Some(ch) = channel_opt {
                    let _ = ch.send(json!({
                        "type": "tick",
                        "positionMs": position_ms,
                        "durationMs": duration_ms,
                        "status": status,
                        "rate": rate,
                        "trackId": track_id,
                    }));
                } else {
                    emit_bridge(app, "position_changed", json!(position_ms));
                }
                if dur_changed && duration_ms > 0.0 {
                    emit_bridge(app, "duration_changed", json!(duration_ms));
                }
            }
            NativeAudioEvent::State { status, .. } => {
                {
                    let mut inner = self.inner.lock().unwrap();
                    inner.state = status.clone();
                }
                emit_bridge(app, "playback_state_changed", json!(status));
            }
            NativeAudioEvent::Ended { .. } => {
                self.next_track(app, db, true);
            }
            NativeAudioEvent::Remote {
                action,
                position_ms,
            } => match action.as_str() {
                "play" => self.play(app, db),
                "pause" => self.pause(app),
                "next" => self.next_track(app, db, false),
                "prev" => self.prev_track(app, db),
                "seek" => {
                    if let Some(ms) = position_ms {
                        let mut inner = self.inner.lock().unwrap();
                        inner.position_ms = ms;
                        drop(inner);
                        emit_bridge(app, "position_changed", json!(ms));
                    }
                }
                _ => {}
            },
            NativeAudioEvent::Error { message, track_id } => {
                emit_bridge(
                    app,
                    "playback_error",
                    json!(json!({ "trackId": track_id, "error": message }).to_string()),
                );
            }
        }
    }
}

fn resolve_playable_source(db: &LibraryDb, track: &Track) -> (String, HashMap<String, String>) {
    let mut headers = HashMap::new();
    match track.source.as_str() {
        "subsonic" => {
            if let (Some(sid), Some(song_id)) = (track.server_id, track.subsonic_id.as_deref()) {
                if let Ok(Some(srv)) = db.get_subsonic_server(sid) {
                    return (subsonic::build_stream_url(&srv, song_id), headers);
                }
            }
            (track.path.clone(), headers)
        }
        "webdav" => {
            if let Some(sid) = track.server_id {
                if let Ok(Some(srv)) = db.get_webdav_server(sid) {
                    if !srv.username.is_empty() || !srv.password.is_empty() {
                        headers.insert(
                            "Authorization".to_string(),
                            webdav::basic_auth_header(&srv.username, &srv.password),
                        );
                    }
                }
            }
            (track.path.clone(), headers)
        }
        _ => (track.path.clone(), headers),
    }
}
