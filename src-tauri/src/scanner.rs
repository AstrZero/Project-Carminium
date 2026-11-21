use crate::cover;
use crate::db::{md5_hex, now_secs, LibraryDb, Track};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::ItemKey;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "m4a", "aac", "wav", "alac", "aif", "aiff", "ogg", "opus", "caf", "mp4",
];

pub fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn scan_local_folder(db: &LibraryDb, folder_path: &Path) -> Result<(usize, usize), String> {
    if !folder_path.exists() {
        let _ = fs::create_dir_all(folder_path);
    }
    let folder_str = folder_path.to_string_lossy().to_string();
    let folder_id = db
        .ensure_folder(&folder_str)
        .map_err(|e| format!("ensure_folder failed: {e}"))?;

    let mut present_ids = HashSet::new();
    let mut scanned = 0usize;

    for entry in WalkDir::new(folder_path)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if !p.is_file() || !is_audio_file(p) {
            continue;
        }
        let abs_str = p.to_string_lossy().to_string();
        let tid = md5_hex(&abs_str);
        present_ids.insert(tid.clone());

        let file_size = fs::metadata(p).map(|m| m.len() as i64).unwrap_or(0);
        let suffix = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase());
        let fallback_title = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();

        let mut title = fallback_title;
        let mut artist = String::new();
        let mut album = String::new();
        let mut album_artist = String::new();
        let mut track_number = None;
        let mut disc_number = None;
        let mut year = None;
        let mut duration_ms = 0i64;
        let mut has_cover = 0i64;
        let mut lyrics = None;
        let mut genre = None;

        if let Ok(probe) = Probe::open(p) {
            if let Ok(tagged) = probe.guess_file_type().and_then(|pr| pr.read()) {
                let props = tagged.properties();
                duration_ms = props.duration().as_millis() as i64;

                if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
                    if let Some(t) = tag.title() {
                        if !t.trim().is_empty() {
                            title = t.trim().to_string();
                        }
                    }
                    if let Some(a) = tag.artist() {
                        artist = a.trim().to_string();
                    }
                    if let Some(al) = tag.album() {
                        album = al.trim().to_string();
                    }
                    if let Some(aa) = tag.get_string(&ItemKey::AlbumArtist) {
                        album_artist = aa.trim().to_string();
                    }
                    track_number = tag.track().map(|n| n as i64);
                    disc_number = tag.disk().map(|n| n as i64);
                    year = tag.year().map(|y| y as i64);
                    genre = tag.genre().map(|g| g.trim().to_string());
                    if let Some(lrc) = tag.get_string(&ItemKey::Lyrics) {
                        if !lrc.trim().is_empty() {
                            lyrics = Some(lrc.to_string());
                        }
                    }
                    if let Some(pic) = tag.pictures().first() {
                        has_cover = 1;
                        cover::save_cover_bytes(&db.cover_cache_dir, &tid, pic.data());
                    }
                }
            }
        }

        // Fallback to sidecar .lrc file if no embedded lyrics
        if lyrics.is_none() {
            let lrc_path = p.with_extension("lrc");
            if let Ok(content) = fs::read_to_string(lrc_path) {
                if !content.trim().is_empty() {
                    lyrics = Some(content);
                }
            }
        }

        let sort_key = title.trim().to_lowercase();
        let track = Track {
            id: tid,
            path: abs_str,
            folder_id: Some(folder_id),
            title,
            artist,
            album,
            album_artist,
            track_number,
            disc_number,
            year,
            duration_ms,
            file_size,
            has_cover,
            lyrics,
            alias_title: None,
            added_at: now_secs(),
            source: "local".to_string(),
            server_id: None,
            subsonic_id: None,
            cover_id: None,
            suffix,
            genre,
            sort_key,
        };
        if db.upsert_track(&track).is_ok() {
            scanned += 1;
        }
    }

    let removed = db
        .prune_folder_missing_tracks(folder_id, &present_ids)
        .unwrap_or(0);
    db.touch_folder_scan(folder_id);
    Ok((scanned, removed))
}

pub fn read_embedded_lyrics(file_path: &Path) -> Option<String> {
    if let Ok(probe) = Probe::open(file_path) {
        if let Ok(tagged) = probe.guess_file_type().and_then(|pr| pr.read()) {
            if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
                if let Some(lrc) = tag.get_string(&ItemKey::Lyrics) {
                    if !lrc.trim().is_empty() {
                        return Some(lrc.to_string());
                    }
                }
            }
        }
    }
    let lrc_path = file_path.with_extension("lrc");
    fs::read_to_string(lrc_path)
        .ok()
        .filter(|s| !s.trim().is_empty())
}
