use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn md5_hex(input: &str) -> String {
    format!("{:x}", md5::compute(input.as_bytes()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub path: String,
    pub folder_id: Option<i64>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub year: Option<i64>,
    pub duration_ms: i64,
    pub file_size: i64,
    pub has_cover: i64,
    pub lyrics: Option<String>,
    pub alias_title: Option<String>,
    pub added_at: f64,
    pub source: String,
    pub server_id: Option<i64>,
    pub subsonic_id: Option<String>,
    pub cover_id: Option<String>,
    pub suffix: Option<String>,
    pub genre: Option<String>,
    pub sort_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsonicServer {
    pub id: i64,
    pub name: String,
    pub server_url: String,
    pub username: String,
    pub password: String,
    pub protocol_mode: String,
    pub added_at: f64,
    pub last_sync: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebDavServer {
    pub id: i64,
    pub name: String,
    pub server_url: String,
    pub username: String,
    pub password: String,
    pub added_at: f64,
    pub last_sync: Option<f64>,
}

pub struct LibraryDb {
    conn: Mutex<Connection>,
    pub cover_cache_dir: PathBuf,
    pub artist_image_dir: PathBuf,
}

fn make_sort_key(text: &str) -> String {
    text.trim().to_lowercase()
}

fn row_to_track(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    let title: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
    let artist: String = row.get::<_, Option<String>>(4)?.unwrap_or_default();
    let album: String = row.get::<_, Option<String>>(5)?.unwrap_or_default();
    let album_artist: String = row.get::<_, Option<String>>(6)?.unwrap_or_default();
    let sort_key = make_sort_key(&title);
    Ok(Track {
        id: row.get(0)?,
        path: row.get(1)?,
        folder_id: row.get(2)?,
        title,
        artist,
        album,
        album_artist,
        track_number: row.get(7)?,
        disc_number: row.get(8)?,
        year: row.get(9)?,
        duration_ms: row.get::<_, Option<i64>>(10)?.unwrap_or(0),
        file_size: row.get::<_, Option<i64>>(11)?.unwrap_or(0),
        has_cover: row.get::<_, Option<i64>>(12)?.unwrap_or(0),
        lyrics: row.get(13)?,
        alias_title: row.get(14)?,
        added_at: row.get::<_, Option<f64>>(15)?.unwrap_or(0.0),
        source: row
            .get::<_, Option<String>>(16)?
            .unwrap_or_else(|| "local".to_string()),
        server_id: row.get(17)?,
        subsonic_id: row.get(18)?,
        cover_id: row.get(19)?,
        suffix: row.get(20)?,
        genre: row.get(21)?,
        sort_key,
    })
}

const TRACK_SELECT: &str = "SELECT id, path, folder_id, title, artist, album, album_artist, \
    track_number, disc_number, year, duration_ms, file_size, has_cover, lyrics, alias_title, \
    added_at, source, server_id, subsonic_id, cover_id, suffix, genre FROM tracks";

impl LibraryDb {
    pub fn open(data_dir: &Path, cache_dir: &Path) -> rusqlite::Result<Self> {
        let _ = std::fs::create_dir_all(data_dir);
        let cover_cache_dir = cache_dir.join("cover_cache");
        let artist_image_dir = cache_dir.join("artist_images");
        let _ = std::fs::create_dir_all(&cover_cache_dir);
        let _ = std::fs::create_dir_all(&artist_image_dir);

        let db_path = data_dir.join("library.db");
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;

             CREATE TABLE IF NOT EXISTS folders (
                 id        INTEGER PRIMARY KEY AUTOINCREMENT,
                 path      TEXT UNIQUE NOT NULL,
                 added_at  REAL NOT NULL,
                 last_scan REAL,
                 kind      TEXT DEFAULT 'folder'
             );

             CREATE TABLE IF NOT EXISTS tracks (
                 id           TEXT PRIMARY KEY,
                 path         TEXT UNIQUE NOT NULL,
                 folder_id    INTEGER REFERENCES folders(id) ON DELETE CASCADE,
                 title        TEXT,
                 artist       TEXT,
                 album        TEXT,
                 album_artist TEXT,
                 track_number INTEGER,
                 disc_number  INTEGER,
                 year         INTEGER,
                 duration_ms  INTEGER,
                 file_size    INTEGER,
                 has_cover    INTEGER DEFAULT 0,
                 lyrics       TEXT,
                 alias_title  TEXT,
                 added_at     REAL NOT NULL,
                 source       TEXT DEFAULT 'local',
                 server_id    INTEGER,
                 subsonic_id  TEXT,
                 cover_id     TEXT,
                 suffix       TEXT,
                 genre        TEXT
             );

             CREATE TABLE IF NOT EXISTS liked_tracks (
                 track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
                 liked_at REAL NOT NULL
             );

             CREATE TABLE IF NOT EXISTS liked_artists (
                 name     TEXT PRIMARY KEY,
                 liked_at REAL NOT NULL
             );

             CREATE TABLE IF NOT EXISTS liked_albums (
                 album        TEXT NOT NULL,
                 album_artist TEXT NOT NULL DEFAULT '',
                 liked_at     REAL NOT NULL,
                 PRIMARY KEY (album, album_artist)
             );

             CREATE TABLE IF NOT EXISTS play_history (
                 id        INTEGER PRIMARY KEY AUTOINCREMENT,
                 track_id  TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                 played_at REAL NOT NULL
             );

             CREATE TABLE IF NOT EXISTS playlists (
                 id             INTEGER PRIMARY KEY AUTOINCREMENT,
                 name           TEXT NOT NULL,
                 created_at     REAL NOT NULL,
                 updated_at     REAL NOT NULL,
                 source         TEXT DEFAULT 'local',
                 server_id      INTEGER,
                 remote_id      TEXT,
                 remote_changed TEXT,
                 cover_art_id   TEXT,
                 owner          TEXT,
                 owner_email    TEXT
             );

             CREATE TABLE IF NOT EXISTS playlist_tracks (
                 playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                 track_id    TEXT NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                 position    INTEGER NOT NULL,
                 added_at    REAL NOT NULL,
                 PRIMARY KEY (playlist_id, position)
             );

             CREATE TABLE IF NOT EXISTS subsonic_sources (
                 id            INTEGER PRIMARY KEY AUTOINCREMENT,
                 name          TEXT NOT NULL,
                 server_url    TEXT NOT NULL,
                 username      TEXT NOT NULL,
                 password      TEXT NOT NULL,
                 protocol_mode TEXT NOT NULL DEFAULT 'subsonic',
                 added_at      REAL NOT NULL,
                 last_sync     REAL
             );

             CREATE TABLE IF NOT EXISTS webdav_sources (
                 id         INTEGER PRIMARY KEY AUTOINCREMENT,
                 name       TEXT NOT NULL,
                 server_url TEXT NOT NULL,
                 username   TEXT NOT NULL,
                 password   TEXT NOT NULL,
                 added_at   REAL NOT NULL,
                 last_sync  REAL
             );

             CREATE TABLE IF NOT EXISTS cover_colors (
                 track_id TEXT PRIMARY KEY REFERENCES tracks(id) ON DELETE CASCADE,
                 colors   TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS artist_overrides (
                 name       TEXT PRIMARY KEY,
                 aliases    TEXT,
                 bio        TEXT,
                 image_path TEXT,
                 updated_at REAL NOT NULL
             );",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
            cover_cache_dir,
            artist_image_dir,
        })
    }

    // ── Folders ─────────────────────────────────────────────────────

    pub fn ensure_folder(&self, folder_path: &str) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        if let Some(id) = conn
            .query_row(
                "SELECT id FROM folders WHERE path = ?1",
                params![folder_path],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            return Ok(id);
        }
        conn.execute(
            "INSERT INTO folders (path, added_at, last_scan, kind) VALUES (?1, ?2, ?2, 'folder')",
            params![folder_path, now_secs()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn remove_folder(&self, folder_path: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM folders WHERE path = ?1", params![folder_path])?;
        Ok(())
    }

    pub fn get_folders(&self) -> rusqlite::Result<Vec<Value>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT f.id, f.path, f.added_at, f.last_scan, f.kind, \
             (SELECT COUNT(*) FROM tracks t WHERE t.folder_id = f.id) AS track_count \
             FROM folders f ORDER BY f.added_at ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "path": r.get::<_, String>(1)?,
                "added_at": r.get::<_, f64>(2)?,
                "last_scan": r.get::<_, Option<f64>>(3)?,
                "kind": r.get::<_, Option<String>>(4)?.unwrap_or_else(|| "folder".into()),
                "track_count": r.get::<_, i64>(5)?,
            }))
        })?;
        rows.collect()
    }

    pub fn touch_folder_scan(&self, folder_id: i64) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "UPDATE folders SET last_scan = ?1 WHERE id = ?2",
                params![now_secs(), folder_id],
            );
        }
    }

    // ── Tracks ──────────────────────────────────────────────────────

    pub fn upsert_track(&self, t: &Track) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tracks (
                id, path, folder_id, title, artist, album, album_artist,
                track_number, disc_number, year, duration_ms, file_size,
                has_cover, lyrics, alias_title, added_at,
                source, server_id, subsonic_id, cover_id, suffix, genre
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20, ?21, ?22
             )
             ON CONFLICT(id) DO UPDATE SET
                path = excluded.path,
                folder_id = COALESCE(excluded.folder_id, tracks.folder_id),
                title = excluded.title,
                artist = excluded.artist,
                album = excluded.album,
                album_artist = excluded.album_artist,
                track_number = excluded.track_number,
                disc_number = excluded.disc_number,
                year = excluded.year,
                duration_ms = excluded.duration_ms,
                file_size = excluded.file_size,
                has_cover = excluded.has_cover,
                lyrics = COALESCE(tracks.lyrics, excluded.lyrics),
                alias_title = COALESCE(excluded.alias_title, tracks.alias_title),
                source = excluded.source,
                server_id = excluded.server_id,
                subsonic_id = excluded.subsonic_id,
                cover_id = excluded.cover_id,
                suffix = excluded.suffix,
                genre = excluded.genre",
            params![
                t.id,
                t.path,
                t.folder_id,
                t.title,
                t.artist,
                t.album,
                t.album_artist,
                t.track_number,
                t.disc_number,
                t.year,
                t.duration_ms,
                t.file_size,
                t.has_cover,
                t.lyrics,
                t.alias_title,
                t.added_at,
                t.source,
                t.server_id,
                t.subsonic_id,
                t.cover_id,
                t.suffix,
                t.genre,
            ],
        )?;
        Ok(())
    }

    pub fn prune_folder_missing_tracks(
        &self,
        folder_id: i64,
        present_ids: &HashSet<String>,
    ) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id FROM tracks WHERE folder_id = ?1")?;
        let existing: Vec<String> = stmt
            .query_map(params![folder_id], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        let mut removed = 0usize;
        for id in existing {
            if !present_ids.contains(&id) {
                conn.execute("DELETE FROM tracks WHERE id = ?1", params![id])?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn replace_remote_tracks(
        &self,
        source: &str,
        server_id: i64,
        tracks: &[Track],
    ) -> rusqlite::Result<usize> {
        let present: HashSet<String> = tracks.iter().map(|t| t.id.clone()).collect();
        for t in tracks {
            self.upsert_track(t)?;
        }
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id FROM tracks WHERE source = ?1 AND server_id = ?2",
        )?;
        let existing: Vec<String> = stmt
            .query_map(params![source, server_id], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        for id in existing {
            if !present.contains(&id) {
                conn.execute("DELETE FROM tracks WHERE id = ?1", params![id])?;
            }
        }
        Ok(tracks.len())
    }

    pub fn get_all_tracks(&self) -> rusqlite::Result<Vec<Track>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!("{TRACK_SELECT} ORDER BY LOWER(COALESCE(title, '')) ASC");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], row_to_track)?;
        rows.collect()
    }

    pub fn get_track(&self, id: &str) -> rusqlite::Result<Option<Track>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!("{TRACK_SELECT} WHERE id = ?1");
        conn.query_row(&sql, params![id], row_to_track).optional()
    }

    pub fn update_lyrics(&self, track_id: &str, lyrics: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE tracks SET lyrics = ?1 WHERE id = ?2",
            params![lyrics, track_id],
        )?;
        Ok(())
    }

    pub fn search_tracks(&self, query: &str) -> rusqlite::Result<Vec<Track>> {
        let q = format!("%{}%", query.trim().to_lowercase());
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "{TRACK_SELECT} WHERE LOWER(COALESCE(title,'')) LIKE ?1 \
             OR LOWER(COALESCE(artist,'')) LIKE ?1 \
             OR LOWER(COALESCE(album,'')) LIKE ?1 \
             OR LOWER(COALESCE(alias_title,'')) LIKE ?1 \
             ORDER BY LOWER(COALESCE(title,'')) ASC LIMIT 500"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![q], row_to_track)?;
        rows.collect()
    }

    // ── Albums & Artists Aggregation ────────────────────────────────

    pub fn get_albums(&self) -> rusqlite::Result<Vec<Value>> {
        let tracks = self.get_all_tracks()?;
        let mut map: HashMap<(String, String), (i64, Option<i64>, Option<String>, i64, String)> =
            HashMap::new();
        for t in tracks {
            let album = if t.album.trim().is_empty() {
                "未知专辑".to_string()
            } else {
                t.album.trim().to_string()
            };
            let aa = if !t.album_artist.trim().is_empty() {
                t.album_artist.trim().to_string()
            } else if !t.artist.trim().is_empty() {
                t.artist.trim().to_string()
            } else {
                "未知艺术家".to_string()
            };
            let entry = map
                .entry((album.clone(), aa.clone()))
                .or_insert_with(|| (0, t.year, None, 0, aa.clone()));
            entry.0 += 1;
            if entry.1.is_none() {
                entry.1 = t.year;
            }
            if entry.2.is_none() && t.has_cover != 0 {
                entry.2 = Some(t.id.clone());
            }
            entry.3 += t.duration_ms;
        }
        let mut list: Vec<Value> = map
            .into_iter()
            .map(|((album, album_artist), (count, year, cover_track_id, dur, artist))| {
                let sk = make_sort_key(&album);
                json!({
                    "album": album,
                    "album_artist": album_artist,
                    "artist": artist,
                    "track_count": count,
                    "year": year,
                    "cover_track_id": cover_track_id,
                    "total_duration_ms": dur,
                    "sort_key": sk,
                })
            })
            .collect();
        list.sort_by(|a, b| {
            a["sort_key"]
                .as_str()
                .unwrap_or("")
                .cmp(b["sort_key"].as_str().unwrap_or(""))
        });
        Ok(list)
    }

    pub fn get_album_tracks(
        &self,
        album: &str,
        album_artist: Option<&str>,
    ) -> rusqlite::Result<Vec<Track>> {
        let all = self.get_all_tracks()?;
        let target_album = album.trim();
        let target_aa = album_artist.map(|s| s.trim()).filter(|s| !s.is_empty());
        let mut out: Vec<Track> = all
            .into_iter()
            .filter(|t| {
                let al = if t.album.trim().is_empty() {
                    "未知专辑"
                } else {
                    t.album.trim()
                };
                if al != target_album {
                    return false;
                }
                if let Some(aa) = target_aa {
                    let t_aa = if !t.album_artist.trim().is_empty() {
                        t.album_artist.trim()
                    } else if !t.artist.trim().is_empty() {
                        t.artist.trim()
                    } else {
                        "未知艺术家"
                    };
                    t_aa == aa
                } else {
                    true
                }
            })
            .collect();
        out.sort_by_key(|t| (t.disc_number.unwrap_or(1), t.track_number.unwrap_or(9999)));
        Ok(out)
    }

    pub fn get_artists(&self, separators: &str) -> rusqlite::Result<Vec<Value>> {
        let tracks = self.get_all_tracks()?;
        let sep_chars: Vec<char> = separators.chars().filter(|c| !c.is_whitespace()).collect();
        let mut map: HashMap<String, (i64, HashSet<String>, Option<String>)> = HashMap::new();

        for t in tracks {
            let raw = if t.artist.trim().is_empty() {
                "未知艺术家"
            } else {
                t.artist.trim()
            };
            let parts: Vec<String> = if sep_chars.is_empty() {
                vec![raw.to_string()]
            } else {
                raw.split(|c| sep_chars.contains(&c))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            };
            for name in parts {
                let entry = map
                    .entry(name)
                    .or_insert_with(|| (0, HashSet::new(), None));
                entry.0 += 1;
                if !t.album.trim().is_empty() {
                    entry.1.insert(t.album.trim().to_string());
                }
                if entry.2.is_none() && t.has_cover != 0 {
                    entry.2 = Some(t.id.clone());
                }
            }
        }

        let mut list: Vec<Value> = map
            .into_iter()
            .map(|(name, (count, albums, cover_track_id))| {
                let sk = make_sort_key(&name);
                json!({
                    "artist": name,
                    "name": name,
                    "track_count": count,
                    "album_count": albums.len(),
                    "cover_track_id": cover_track_id,
                    "sort_key": sk,
                })
            })
            .collect();
        list.sort_by(|a, b| {
            a["sort_key"]
                .as_str()
                .unwrap_or("")
                .cmp(b["sort_key"].as_str().unwrap_or(""))
        });
        Ok(list)
    }

    pub fn get_artist_tracks(&self, artist: &str, separators: &str) -> rusqlite::Result<Vec<Track>> {
        let tracks = self.get_all_tracks()?;
        let target = artist.trim();
        let sep_chars: Vec<char> = separators.chars().filter(|c| !c.is_whitespace()).collect();
        Ok(tracks
            .into_iter()
            .filter(|t| {
                let raw = if t.artist.trim().is_empty() {
                    "未知艺术家"
                } else {
                    t.artist.trim()
                };
                if raw == target {
                    return true;
                }
                if !sep_chars.is_empty() {
                    raw.split(|c| sep_chars.contains(&c))
                        .any(|s| s.trim() == target)
                } else {
                    false
                }
            })
            .collect())
    }

    // ── Likes ───────────────────────────────────────────────────────

    pub fn is_track_liked(&self, track_id: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT 1 FROM liked_tracks WHERE track_id = ?1",
            params![track_id],
            |_| Ok(true),
        )
        .unwrap_or(false)
    }

    pub fn toggle_liked_track(&self, track_id: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM liked_tracks WHERE track_id = ?1",
                params![track_id],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if exists {
            conn.execute(
                "DELETE FROM liked_tracks WHERE track_id = ?1",
                params![track_id],
            )?;
            Ok(false)
        } else {
            conn.execute(
                "INSERT OR REPLACE INTO liked_tracks (track_id, liked_at) VALUES (?1, ?2)",
                params![track_id, now_secs()],
            )?;
            Ok(true)
        }
    }

    pub fn get_liked_tracks(&self) -> rusqlite::Result<Vec<Track>> {
        let conn = self.conn.lock().unwrap();
        let sql = "SELECT t.id, t.path, t.folder_id, t.title, t.artist, t.album, t.album_artist, \
             t.track_number, t.disc_number, t.year, t.duration_ms, t.file_size, t.has_cover, \
             t.lyrics, t.alias_title, t.added_at, t.source, t.server_id, t.subsonic_id, \
             t.cover_id, t.suffix, t.genre \
             FROM liked_tracks lt JOIN tracks t ON t.id = lt.track_id \
             ORDER BY lt.liked_at DESC";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], row_to_track)?;
        rows.collect()
    }

    pub fn toggle_liked_artist(&self, name: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM liked_artists WHERE name = ?1",
                params![name],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if exists {
            conn.execute("DELETE FROM liked_artists WHERE name = ?1", params![name])?;
            Ok(false)
        } else {
            conn.execute(
                "INSERT OR REPLACE INTO liked_artists (name, liked_at) VALUES (?1, ?2)",
                params![name, now_secs()],
            )?;
            Ok(true)
        }
    }

    pub fn is_artist_liked(&self, name: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT 1 FROM liked_artists WHERE name = ?1",
            params![name],
            |_| Ok(true),
        )
        .unwrap_or(false)
    }

    pub fn get_liked_artists(&self, separators: &str) -> rusqlite::Result<Vec<Value>> {
        let liked_names: Vec<String> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt =
                conn.prepare("SELECT name FROM liked_artists ORDER BY liked_at DESC")?;
            stmt.query_map([], |r| r.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };
        let all = self.get_artists(separators)?;
        let mut by_name: HashMap<String, Value> = all
            .into_iter()
            .filter_map(|v| {
                v["artist"]
                    .as_str()
                    .map(|n| (n.to_string(), v.clone()))
            })
            .collect();
        let mut out = Vec::new();
        for name in liked_names {
            if let Some(v) = by_name.remove(&name) {
                out.push(v);
            } else {
                out.push(json!({
                    "artist": name,
                    "name": name,
                    "track_count": 0,
                    "album_count": 0,
                    "cover_track_id": null,
                    "sort_key": make_sort_key(&name),
                }));
            }
        }
        Ok(out)
    }

    pub fn toggle_liked_album(&self, album: &str, album_artist: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM liked_albums WHERE album = ?1 AND album_artist = ?2",
                params![album, album_artist],
                |_| Ok(true),
            )
            .unwrap_or(false);
        if exists {
            conn.execute(
                "DELETE FROM liked_albums WHERE album = ?1 AND album_artist = ?2",
                params![album, album_artist],
            )?;
            Ok(false)
        } else {
            conn.execute(
                "INSERT OR REPLACE INTO liked_albums (album, album_artist, liked_at) VALUES (?1, ?2, ?3)",
                params![album, album_artist, now_secs()],
            )?;
            Ok(true)
        }
    }

    pub fn is_album_liked(&self, album: &str, album_artist: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT 1 FROM liked_albums WHERE album = ?1 AND album_artist = ?2",
            params![album, album_artist],
            |_| Ok(true),
        )
        .unwrap_or(false)
    }

    pub fn get_liked_albums(&self) -> rusqlite::Result<Vec<Value>> {
        let keys: Vec<(String, String)> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT album, album_artist FROM liked_albums ORDER BY liked_at DESC",
            )?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect()
        };
        let all = self.get_albums()?;
        let mut by_key: HashMap<(String, String), Value> = all
            .into_iter()
            .map(|v| {
                let k = (
                    v["album"].as_str().unwrap_or("").to_string(),
                    v["album_artist"].as_str().unwrap_or("").to_string(),
                );
                (k, v)
            })
            .collect();
        let mut out = Vec::new();
        for k in keys {
            if let Some(v) = by_key.remove(&k) {
                out.push(v);
            }
        }
        Ok(out)
    }

    // ── Play History & Mixes ────────────────────────────────────────

    pub fn record_play(&self, track_id: &str) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "INSERT INTO play_history (track_id, played_at) VALUES (?1, ?2)",
                params![track_id, now_secs()],
            );
        }
    }

    pub fn get_play_history(&self, limit: i64) -> rusqlite::Result<Vec<Value>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT t.id, t.path, t.folder_id, t.title, t.artist, t.album, t.album_artist, \
             t.track_number, t.disc_number, t.year, t.duration_ms, t.file_size, t.has_cover, \
             t.lyrics, t.alias_title, t.added_at, t.source, t.server_id, t.subsonic_id, \
             t.cover_id, t.suffix, t.genre, h.played_at \
             FROM play_history h JOIN tracks t ON t.id = h.track_id \
             ORDER BY h.played_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            let track = row_to_track(r)?;
            let played_at: f64 = r.get(22)?;
            let mut val = serde_json::to_value(track).unwrap_or_else(|_| json!({}));
            if let Some(obj) = val.as_object_mut() {
                obj.insert("played_at".into(), json!(played_at));
            }
            Ok(val)
        })?;
        rows.collect()
    }

    pub fn clear_play_history(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM play_history", [])?;
        Ok(())
    }

    pub fn get_play_stats(&self) -> rusqlite::Result<Value> {
        let conn = self.conn.lock().unwrap();
        let total_plays: i64 =
            conn.query_row("SELECT COUNT(*) FROM play_history", [], |r| r.get(0))?;
        let unique_tracks: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT track_id) FROM play_history",
            [],
            |r| r.get(0),
        )?;
        let total_tracks: i64 =
            conn.query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get(0))?;
        Ok(json!({
            "total_plays": total_plays,
            "unique_tracks": unique_tracks,
            "total_tracks": total_tracks,
        }))
    }

    pub fn get_daily_mixes(&self) -> rusqlite::Result<Vec<Value>> {
        let tracks = self.get_all_tracks()?;
        if tracks.is_empty() {
            return Ok(Vec::new());
        }
        let mut by_artist: HashMap<String, Vec<Track>> = HashMap::new();
        for t in tracks {
            let artist = if t.artist.trim().is_empty() {
                continue;
            } else {
                t.artist.trim().to_string()
            };
            by_artist.entry(artist).or_default().push(t);
        }
        let mut pairs: Vec<(String, Vec<Track>)> = by_artist
            .into_iter()
            .filter(|(_, list)| list.len() >= 2)
            .collect();
        pairs.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        Ok(pairs
            .into_iter()
            .take(6)
            .map(|(artist, list)| {
                let count = list.len();
                json!({
                    "type": "artist",
                    "name": artist,
                    "subtitle": format!("{} 首曲目", count),
                    "playCount": count,
                    "tracks": list,
                })
            })
            .collect())
    }

    pub fn get_explore_tracks(&self, limit: usize) -> rusqlite::Result<Value> {
        let all = self.get_all_tracks()?;
        let subset: Vec<Track> = all.into_iter().take(limit).collect();
        Ok(json!({
            "title": "探索",
            "subtitle": "发现你尚未听过的曲目",
            "tracks": subset,
        }))
    }

    // ── Playlists ───────────────────────────────────────────────────

    pub fn get_playlists(&self) -> rusqlite::Result<Vec<Value>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT p.id, p.name, p.created_at, p.updated_at, p.source, p.server_id, \
             p.remote_id, p.remote_changed, p.cover_art_id, p.owner, \
             (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id) AS track_count, \
             (SELECT pt.track_id FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id \
              WHERE pt.playlist_id = p.id AND t.has_cover = 1 ORDER BY pt.position ASC LIMIT 1) AS cover_track_id \
             FROM playlists p ORDER BY p.updated_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "created_at": r.get::<_, f64>(2)?,
                "updated_at": r.get::<_, f64>(3)?,
                "source": r.get::<_, Option<String>>(4)?.unwrap_or_else(|| "local".into()),
                "server_id": r.get::<_, Option<i64>>(5)?,
                "remote_id": r.get::<_, Option<String>>(6)?,
                "remote_changed": r.get::<_, Option<String>>(7)?,
                "cover_art_id": r.get::<_, Option<String>>(8)?,
                "owner": r.get::<_, Option<String>>(9)?,
                "track_count": r.get::<_, i64>(10)?,
                "cover_track_id": r.get::<_, Option<String>>(11)?,
            }))
        })?;
        rows.collect()
    }

    pub fn create_playlist(&self, name: &str) -> rusqlite::Result<Value> {
        let conn = self.conn.lock().unwrap();
        let now = now_secs();
        conn.execute(
            "INSERT INTO playlists (name, created_at, updated_at, source) VALUES (?1, ?2, ?2, 'local')",
            params![name, now],
        )?;
        let id = conn.last_insert_rowid();
        Ok(json!({
            "id": id,
            "name": name,
            "created_at": now,
            "updated_at": now,
            "source": "local",
            "track_count": 0,
        }))
    }

    pub fn rename_playlist(&self, id: i64, name: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE playlists SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![name, now_secs(), id],
        )?;
        Ok(())
    }

    pub fn delete_playlist(&self, id: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM playlists WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn add_tracks_to_playlist(
        &self,
        playlist_id: i64,
        track_ids: &[String],
    ) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let mut max_pos: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(position), -1) FROM playlist_tracks WHERE playlist_id = ?1",
                params![playlist_id],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        let now = now_secs();
        let mut added = 0usize;
        for tid in track_ids {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
                    params![playlist_id, tid],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            if exists {
                continue;
            }
            max_pos += 1;
            if conn
                .execute(
                    "INSERT INTO playlist_tracks (playlist_id, track_id, position, added_at) VALUES (?1, ?2, ?3, ?4)",
                    params![playlist_id, tid, max_pos, now],
                )
                .is_ok()
            {
                added += 1;
            }
        }
        let _ = conn.execute(
            "UPDATE playlists SET updated_at = ?1 WHERE id = ?2",
            params![now, playlist_id],
        );
        Ok(added)
    }

    pub fn remove_from_playlist(&self, playlist_id: i64, track_id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
            params![playlist_id, track_id],
        )?;
        conn.execute(
            "UPDATE playlists SET updated_at = ?1 WHERE id = ?2",
            params![now_secs(), playlist_id],
        )?;
        Ok(())
    }

    pub fn get_playlist_tracks(&self, playlist_id: i64) -> rusqlite::Result<Vec<Track>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT t.id, t.path, t.folder_id, t.title, t.artist, t.album, t.album_artist, \
             t.track_number, t.disc_number, t.year, t.duration_ms, t.file_size, t.has_cover, \
             t.lyrics, t.alias_title, t.added_at, t.source, t.server_id, t.subsonic_id, \
             t.cover_id, t.suffix, t.genre \
             FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id \
             WHERE pt.playlist_id = ?1 ORDER BY pt.position ASC",
        )?;
        let rows = stmt.query_map(params![playlist_id], row_to_track)?;
        rows.collect()
    }

    pub fn get_all_playlist_track_ids(&self) -> rusqlite::Result<HashMap<String, Vec<String>>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT playlist_id, track_id FROM playlist_tracks ORDER BY position ASC",
        )?;
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows.flatten() {
            map.entry(row.0.to_string()).or_default().push(row.1);
        }
        Ok(map)
    }

    // ── Subsonic & WebDAV Servers ───────────────────────────────────

    pub fn get_subsonic_servers(&self) -> rusqlite::Result<Vec<Value>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, server_url, username, password, protocol_mode, added_at, last_sync, \
             (SELECT COUNT(*) FROM tracks t WHERE t.source = 'subsonic' AND t.server_id = s.id) \
             FROM subsonic_sources s ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "server_url": r.get::<_, String>(2)?,
                "username": r.get::<_, String>(3)?,
                "password": r.get::<_, String>(4)?,
                "protocol_mode": r.get::<_, String>(5)?,
                "added_at": r.get::<_, f64>(6)?,
                "last_sync": r.get::<_, Option<f64>>(7)?,
                "track_count": r.get::<_, i64>(8)?,
                "type": "subsonic",
            }))
        })?;
        rows.collect()
    }

    pub fn get_subsonic_server(&self, id: i64) -> rusqlite::Result<Option<SubsonicServer>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, server_url, username, password, protocol_mode, added_at, last_sync \
             FROM subsonic_sources WHERE id = ?1",
            params![id],
            |r| {
                Ok(SubsonicServer {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    server_url: r.get(2)?,
                    username: r.get(3)?,
                    password: r.get(4)?,
                    protocol_mode: r.get(5)?,
                    added_at: r.get(6)?,
                    last_sync: r.get(7)?,
                })
            },
        )
        .optional()
    }

    pub fn add_subsonic_server(
        &self,
        name: &str,
        server_url: &str,
        username: &str,
        password: &str,
        protocol_mode: &str,
    ) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO subsonic_sources (name, server_url, username, password, protocol_mode, added_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                name,
                server_url.trim_end_matches('/'),
                username,
                password,
                protocol_mode,
                now_secs()
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_subsonic_server(
        &self,
        id: i64,
        name: &str,
        server_url: &str,
        username: &str,
        password: &str,
        protocol_mode: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE subsonic_sources SET name=?1, server_url=?2, username=?3, password=?4, protocol_mode=?5 WHERE id=?6",
            params![name, server_url.trim_end_matches('/'), username, password, protocol_mode, id],
        )?;
        Ok(())
    }

    pub fn remove_subsonic_server(&self, id: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM tracks WHERE source = 'subsonic' AND server_id = ?1",
            params![id],
        )?;
        conn.execute("DELETE FROM subsonic_sources WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn touch_subsonic_sync(&self, id: i64) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "UPDATE subsonic_sources SET last_sync = ?1 WHERE id = ?2",
                params![now_secs(), id],
            );
        }
    }

    pub fn get_webdav_servers(&self) -> rusqlite::Result<Vec<Value>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, server_url, username, password, added_at, last_sync, \
             (SELECT COUNT(*) FROM tracks t WHERE t.source = 'webdav' AND t.server_id = w.id) \
             FROM webdav_sources w ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "server_url": r.get::<_, String>(2)?,
                "username": r.get::<_, String>(3)?,
                "password": r.get::<_, String>(4)?,
                "added_at": r.get::<_, f64>(5)?,
                "last_sync": r.get::<_, Option<f64>>(6)?,
                "track_count": r.get::<_, i64>(7)?,
                "type": "webdav",
            }))
        })?;
        rows.collect()
    }

    pub fn get_webdav_server(&self, id: i64) -> rusqlite::Result<Option<WebDavServer>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, server_url, username, password, added_at, last_sync \
             FROM webdav_sources WHERE id = ?1",
            params![id],
            |r| {
                Ok(WebDavServer {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    server_url: r.get(2)?,
                    username: r.get(3)?,
                    password: r.get(4)?,
                    added_at: r.get(5)?,
                    last_sync: r.get(6)?,
                })
            },
        )
        .optional()
    }

    pub fn add_webdav_server(
        &self,
        name: &str,
        server_url: &str,
        username: &str,
        password: &str,
    ) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO webdav_sources (name, server_url, username, password, added_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![name, server_url.trim_end_matches('/'), username, password, now_secs()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_webdav_server(
        &self,
        id: i64,
        name: &str,
        server_url: &str,
        username: &str,
        password: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE webdav_sources SET name=?1, server_url=?2, username=?3, password=?4 WHERE id=?5",
            params![name, server_url.trim_end_matches('/'), username, password, id],
        )?;
        Ok(())
    }

    pub fn remove_webdav_server(&self, id: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM tracks WHERE source = 'webdav' AND server_id = ?1",
            params![id],
        )?;
        conn.execute("DELETE FROM webdav_sources WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn touch_webdav_sync(&self, id: i64) {
        if let Ok(conn) = self.conn.lock() {
            let _ = conn.execute(
                "UPDATE webdav_sources SET last_sync = ?1 WHERE id = ?2",
                params![now_secs(), id],
            );
        }
    }

    // ── Cover Colors & Artist Overrides ─────────────────────────────

    pub fn store_cover_colors(&self, track_id: &str, colors_json: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO cover_colors (track_id, colors) VALUES (?1, ?2)",
            params![track_id, colors_json],
        )?;
        Ok(())
    }

    pub fn get_cover_colors(&self, track_ids: &[String]) -> rusqlite::Result<HashMap<String, Value>> {
        let conn = self.conn.lock().unwrap();
        let mut out = HashMap::new();
        let mut stmt = conn.prepare("SELECT colors FROM cover_colors WHERE track_id = ?1")?;
        for tid in track_ids {
            if let Some(raw) = stmt
                .query_row(params![tid], |r| r.get::<_, String>(0))
                .optional()?
            {
                if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                    out.insert(tid.clone(), val);
                }
            }
        }
        Ok(out)
    }

    pub fn get_artist_override(&self, name: &str) -> rusqlite::Result<Value> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT aliases, bio, image_path, updated_at FROM artist_overrides WHERE name = ?1",
                params![name],
                |r| {
                    let aliases: Option<String> = r.get(0)?;
                    let bio: Option<String> = r.get(1)?;
                    let image_path: Option<String> = r.get(2)?;
                    let updated_at: f64 = r.get(3)?;
                    let aliases_val = aliases
                        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                        .unwrap_or(Value::Null);
                    Ok(json!({
                        "name": name,
                        "aliases": aliases_val,
                        "bio": bio,
                        "image_path": image_path.unwrap_or_default(),
                        "updated_at": updated_at,
                    }))
                },
            )
            .optional()?;
        Ok(row.unwrap_or_else(|| {
            json!({
                "name": name,
                "aliases": null,
                "bio": null,
                "image_path": "",
                "updated_at": 0,
            })
        }))
    }

    pub fn save_artist_override(
        &self,
        name: &str,
        aliases: Option<Value>,
        bio: Option<String>,
    ) -> rusqlite::Result<Value> {
        let conn = self.conn.lock().unwrap();
        let now = now_secs();
        let aliases_str = aliases.map(|v| v.to_string());
        conn.execute(
            "INSERT INTO artist_overrides (name, aliases, bio, updated_at) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(name) DO UPDATE SET \
                aliases = COALESCE(excluded.aliases, artist_overrides.aliases), \
                bio = COALESCE(excluded.bio, artist_overrides.bio), \
                updated_at = excluded.updated_at",
            params![name, aliases_str, bio, now],
        )?;
        drop(conn);
        self.get_artist_override(name)
    }
}
