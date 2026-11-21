use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub struct AppSettings {
    path: PathBuf,
    data: Mutex<Map<String, Value>>,
}

fn default_settings_map() -> Map<String, Value> {
    let val = json!({
        "volume": 100,
        "shuffle": false,
        "repeat": "off",
        "theme": "system",
        "music_folders": [],
        "playback_state": {},
        "audio_output_device": "",
        "wasapi_exclusive": false,
        "automix": false,
        "radical_transitions": false,
        "lyrics_progressive_blur": false,
        "lyrics_center": false,
        "lyrics_font_size": 16,
        "lyrics_font": "",
        "lyrics_jp_font": "",
        "lyrics_jp_use_distinct": false,
        "lyrics_credit_filters": "作词,作曲,编曲,和声,对唱,配唱制作人,钢琴,吉他,鼓,贝斯,制作人,制作,混音,混音师,混音室,母带,录音,录音师,录音室,监制,策划,发行,词曲,填词,谱曲,OP,ED,SP",
        "resume_playback": false,
        "circular_cover": false,
        "wave_progress": true,
        "video_background": false,
        "ui_font": "",
        "ui_jp_font": false,
        "mini_player_lyrics": true,
        "np_background_mode": "cover",
        "color_scheme": "tonal_spot",
        "color_spec": "2021",
        "monet_source": "album_cover",
        "language": "zh-CN",
        "smtc_lyrics": false,
        "taskbar_thumb_controls": false,
        "track_notify": false,
        "taskbar_lyrics": false,
        "artist_separators": ";",
        "album_smart_grouping": true,
        "tag_editor_path": "",
        "cache_dir": "",
        "stream_cache_enabled": false,
        "stream_cache_limit_mb": 2048,
        "discord_rpc": false,
        "eq_enabled": false,
        "eq_bands": [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        "dynamic_bass": false,
        "compressor_enabled": false,
        "vocal_enhance": false,
        "guitar_friendly": false,
        "hearing_protection": false,
        "volume_normalization": false,
        "volume_normalization_target": "-14",
        "onboarding_complete": true,
        "memory_optimization": "normal",
        "spatial_audio": false,
        "vbe_enabled": false,
        "library_auto_refresh": false,
        "library_auto_watch": false,
        "subsonic_auto_sync": true,
        "subsonic_sync_interval_minutes": 30,
        "np_secondary_actions": ["shuffle", "repeat", "like", "airplay"],
        "ios_nav_items": ["your_mix", "music", "albums", "settings"],
        "ios_nav_more_style": "popup",
        "show_quality": true,
        "shortcuts": {}
    });
    val.as_object().cloned().unwrap_or_default()
}

impl AppSettings {
    pub fn load(data_dir: &Path) -> Self {
        let _ = fs::create_dir_all(data_dir);
        let path = data_dir.join("settings.json");
        let mut map = default_settings_map();

        if let Ok(raw) = fs::read_to_string(&path) {
            if let Ok(Value::Object(stored)) = serde_json::from_str::<Value>(&raw) {
                for (k, v) in stored {
                    map.insert(k, v);
                }
            }
        }

        // Normalize legacy desktop-only `audio_mode` slot to `airplay` on iOS
        if let Some(Value::Array(arr)) = map.get_mut("np_secondary_actions") {
            if arr.is_empty() {
                *arr = vec![
                    json!("shuffle"),
                    json!("repeat"),
                    json!("like"),
                    json!("airplay"),
                ];
            } else {
                for item in arr.iter_mut() {
                    if item.as_str() == Some("audio_mode") {
                        *item = json!("airplay");
                    }
                }
            }
        }

        Self {
            path,
            data: Mutex::new(map),
        }
    }

    pub fn all(&self) -> Value {
        let guard = self.data.lock().unwrap();
        Value::Object(guard.clone())
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let guard = self.data.lock().unwrap();
        guard.get(key).cloned()
    }

    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    pub fn get_str(&self, key: &str, default: &str) -> String {
        self.get(key)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| default.to_string())
    }

    pub fn get_f64(&self, key: &str, default: f64) -> f64 {
        self.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
    }

    pub fn set(&self, key: &str, value: Value) {
        let mut guard = self.data.lock().unwrap();
        guard.insert(key.to_string(), value);
        self.save_locked(&guard);
    }

    pub fn update(&self, patch: &Map<String, Value>) {
        let mut guard = self.data.lock().unwrap();
        for (k, v) in patch {
            guard.insert(k.clone(), v.clone());
        }
        self.save_locked(&guard);
    }

    fn save_locked(&self, map: &Map<String, Value>) {
        let tmp = self.path.with_extension("json.tmp");
        if let Ok(text) = serde_json::to_string_pretty(&Value::Object(map.clone())) {
            if fs::write(&tmp, text).is_ok() {
                let _ = fs::rename(&tmp, &self.path);
            }
        }
    }
}
