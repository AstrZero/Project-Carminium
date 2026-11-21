use crate::db::LibraryDb;
use crate::subsonic;
use image::ImageFormat;
use lofty::prelude::*;
use lofty::probe::Probe;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

pub fn cover_cache_path(cache_dir: &Path, track_id: &str) -> PathBuf {
    cache_dir.join(format!("{track_id}.jpg"))
}

pub fn cover_sized_path(cache_dir: &Path, track_id: &str, size: u32) -> PathBuf {
    cache_dir.join(format!("{track_id}_{size}.jpg"))
}

pub fn save_cover_bytes(cache_dir: &Path, track_id: &str, raw: &[u8]) -> Option<PathBuf> {
    let _ = fs::create_dir_all(cache_dir);
    let dest = cover_cache_path(cache_dir, track_id);
    if dest.exists() {
        return Some(dest);
    }
    // Re-encode up to 1024px JPEG so MPNowPlayingInfoCenter & WebView load reliably
    if let Ok(img) = image::load_from_memory(raw) {
        let resized = if img.width() > 1024 || img.height() > 1024 {
            img.thumbnail(1024, 1024)
        } else {
            img
        };
        let mut buf = Cursor::new(Vec::new());
        if resized.write_to(&mut buf, ImageFormat::Jpeg).is_ok() {
            if fs::write(&dest, buf.into_inner()).is_ok() {
                return Some(dest);
            }
        }
    }
    if fs::write(&dest, raw).is_ok() {
        return Some(dest);
    }
    None
}

pub async fn resolve_cover_bytes(
    db: &LibraryDb,
    track_id: &str,
    size: Option<u32>,
) -> Option<(Vec<u8>, &'static str)> {
    let base_path = cover_cache_path(&db.cover_cache_dir, track_id);

    if let Some(sz) = size {
        let sized = cover_sized_path(&db.cover_cache_dir, track_id, sz);
        if let Ok(bytes) = fs::read(&sized) {
            return Some((bytes, "image/jpeg"));
        }
    }

    if !base_path.exists() {
        if let Ok(Some(track)) = db.get_track(track_id) {
            match track.source.as_str() {
                "local" => {
                    let p = Path::new(&track.path);
                    if let Ok(probe) = Probe::open(p) {
                        if let Ok(tagged) = probe.guess_file_type().and_then(|pr| pr.read()) {
                            if let Some(tag) =
                                tagged.primary_tag().or_else(|| tagged.first_tag())
                            {
                                if let Some(pic) = tag.pictures().first() {
                                    save_cover_bytes(&db.cover_cache_dir, track_id, pic.data());
                                }
                            }
                        }
                    }
                }
                "subsonic" => {
                    if let (Some(sid), Some(cid)) = (track.server_id, track.cover_id.as_deref()) {
                        if let Ok(Some(srv)) = db.get_subsonic_server(sid) {
                            if let Ok(bytes) = subsonic::fetch_cover_art(&srv, cid, 600).await {
                                save_cover_bytes(&db.cover_cache_dir, track_id, &bytes);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let raw = fs::read(&base_path).ok()?;
    if let Some(sz) = size {
        let clamped = sz.clamp(64, 1024);
        if let Ok(img) = image::load_from_memory(&raw) {
            let thumb = img.thumbnail(clamped, clamped);
            let mut out = Cursor::new(Vec::new());
            if thumb.write_to(&mut out, ImageFormat::Jpeg).is_ok() {
                let bytes = out.into_inner();
                let _ = fs::write(cover_sized_path(&db.cover_cache_dir, track_id, clamped), &bytes);
                return Some((bytes, "image/jpeg"));
            }
        }
    }
    Some((raw, "image/jpeg"))
}

pub fn ensure_artwork_path_sync(db: &LibraryDb, track_id: &str) -> Option<String> {
    let base_path = cover_cache_path(&db.cover_cache_dir, track_id);
    if base_path.exists() {
        return Some(base_path.to_string_lossy().to_string());
    }
    if let Ok(Some(track)) = db.get_track(track_id) {
        if track.source == "local" {
            let p = Path::new(&track.path);
            if let Ok(probe) = Probe::open(p) {
                if let Ok(tagged) = probe.guess_file_type().and_then(|pr| pr.read()) {
                    if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
                        if let Some(pic) = tag.pictures().first() {
                            if let Some(saved) =
                                save_cover_bytes(&db.cover_cache_dir, track_id, pic.data())
                            {
                                return Some(saved.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

pub fn clear_cover_cache(cache_dir: &Path) -> (usize, u64) {
    let mut count = 0usize;
    let mut bytes = 0u64;
    if let Ok(entries) = fs::read_dir(cache_dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    bytes += meta.len();
                    if fs::remove_file(entry.path()).is_ok() {
                        count += 1;
                    }
                }
            }
        }
    }
    (count, bytes)
}

pub fn cache_stats(cache_dir: &Path) -> (usize, u64) {
    let mut count = 0usize;
    let mut bytes = 0u64;
    if let Ok(entries) = fs::read_dir(cache_dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    count += 1;
                    bytes += meta.len();
                }
            }
        }
    }
    (count, bytes)
}
