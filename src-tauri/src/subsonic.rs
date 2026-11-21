use crate::db::{now_secs, LibraryDb, SubsonicServer, Track};
use rand::Rng;
use serde_json::{json, Value};
use std::time::Duration;

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(20))
        .user_agent("ProjectCarminium/iOS")
        .build()
        .map_err(|e| format!("http client init failed: {e}"))
}

fn auth_query(srv: &SubsonicServer) -> String {
    let mut rng = rand::thread_rng();
    let salt: String = (0..12)
        .map(|_| {
            let idx = rng.gen_range(0..36);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + (idx - 10)) as char
            }
        })
        .collect();
    let token = format!(
        "{:x}",
        md5::compute(format!("{}{}", srv.password, salt).as_bytes())
    );
    format!(
        "u={}&t={}&s={}&v=1.16.1&c=Carminium&f=json",
        urlencoding::encode(&srv.username),
        token,
        salt
    )
}

fn endpoint_url(srv: &SubsonicServer, endpoint: &str, extra: &str) -> String {
    let base = srv.server_url.trim_end_matches('/');
    let auth = auth_query(srv);
    if extra.is_empty() {
        format!("{base}/rest/{endpoint}.view?{auth}")
    } else {
        format!("{base}/rest/{endpoint}.view?{auth}&{extra}")
    }
}

pub fn build_stream_url(srv: &SubsonicServer, song_id: &str) -> String {
    endpoint_url(
        srv,
        "stream",
        &format!("id={}", urlencoding::encode(song_id)),
    )
}

async fn call_json(srv: &SubsonicServer, endpoint: &str, extra: &str) -> Result<Value, String> {
    let client = http_client()?;
    let url = endpoint_url(srv, endpoint, extra);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("网络请求失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析服务器响应失败: {e}"))?;
    let sub = body
        .get("subsonic-response")
        .ok_or_else(|| "响应缺少 subsonic-response".to_string())?;
    if sub.get("status").and_then(|v| v.as_str()) != Some("ok") {
        let msg = sub
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or("Subsonic 服务器返回错误");
        return Err(msg.to_string());
    }
    Ok(sub.clone())
}

pub async fn test_server(srv: &SubsonicServer) -> Result<Value, String> {
    let sub = call_json(srv, "ping", "").await?;
    let version = sub
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("1.16.1");
    let server_type = sub
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("subsonic");
    Ok(json!({
        "ok": true,
        "version": version,
        "type": server_type,
    }))
}

pub async fn fetch_cover_art(
    srv: &SubsonicServer,
    cover_id: &str,
    size: u32,
) -> Result<Vec<u8>, String> {
    let client = http_client()?;
    let url = endpoint_url(
        srv,
        "getCoverArt",
        &format!("id={}&size={size}", urlencoding::encode(cover_id)),
    );
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("getCoverArt failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("read cover bytes failed: {e}"))?;
    Ok(bytes.to_vec())
}

pub async fn sync_server(db: &LibraryDb, srv: &SubsonicServer) -> Result<Value, String> {
    let mut album_ids: Vec<String> = Vec::new();
    let mut offset = 0usize;
    let page_size = 500usize;

    loop {
        let sub = call_json(
            srv,
            "getAlbumList2",
            &format!("type=alphabeticalByName&size={page_size}&offset={offset}"),
        )
        .await?;
        let albums = sub
            .pointer("/albumList2/album")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if albums.is_empty() {
            break;
        }
        let batch_len = albums.len();
        for a in albums {
            if let Some(id) = a.get("id").and_then(|v| v.as_str()) {
                album_ids.push(id.to_string());
            }
        }
        if batch_len < page_size {
            break;
        }
        offset += page_size;
    }

    let now = now_secs();
    let mut all_tracks: Vec<Track> = Vec::new();

    for aid in &album_ids {
        let Ok(sub) = call_json(srv, "getAlbum", &format!("id={}", urlencoding::encode(aid))).await
        else {
            continue;
        };
        let album_artist_fallback = sub
            .pointer("/album/artist")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let album_name_fallback = sub
            .pointer("/album/name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let songs = sub
            .pointer("/album/song")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for s in songs {
            let Some(song_id) = s.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let title = s
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();
            let artist = s
                .get("artist")
                .and_then(|v| v.as_str())
                .unwrap_or(&album_artist_fallback)
                .to_string();
            let album = s
                .get("album")
                .and_then(|v| v.as_str())
                .unwrap_or(&album_name_fallback)
                .to_string();
            let album_artist = s
                .get("albumArtist")
                .and_then(|v| v.as_str())
                .unwrap_or(&album_artist_fallback)
                .to_string();
            let duration_ms = s.get("duration").and_then(|v| v.as_i64()).unwrap_or(0) * 1000;
            let file_size = s.get("size").and_then(|v| v.as_i64()).unwrap_or(0);
            let cover_id = s
                .get("coverArt")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string());
            let suffix = s
                .get("suffix")
                .and_then(|v| v.as_str())
                .map(|v| v.to_lowercase());
            let genre = s
                .get("genre")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string());
            let tid = format!("subsonic:{}:{}", srv.id, song_id);
            let sort_key = title.trim().to_lowercase();

            all_tracks.push(Track {
                id: tid.clone(),
                path: format!("subsonic://{}/{}", srv.id, song_id),
                folder_id: None,
                title,
                artist,
                album,
                album_artist,
                track_number: s.get("track").and_then(|v| v.as_i64()),
                disc_number: s.get("discNumber").and_then(|v| v.as_i64()),
                year: s.get("year").and_then(|v| v.as_i64()),
                duration_ms,
                file_size,
                has_cover: if cover_id.is_some() { 1 } else { 0 },
                lyrics: None,
                alias_title: None,
                added_at: now,
                source: "subsonic".to_string(),
                server_id: Some(srv.id),
                subsonic_id: Some(song_id.to_string()),
                cover_id,
                suffix,
                genre,
                sort_key,
            });
        }
    }

    let count = db
        .replace_remote_tracks("subsonic", srv.id, &all_tracks)
        .map_err(|e| format!("保存曲库失败: {e}"))?;
    db.touch_subsonic_sync(srv.id);

    Ok(json!({
        "albums": album_ids.len(),
        "tracks": count,
        "added": count,
    }))
}

pub async fn get_lyrics(
    srv: &SubsonicServer,
    song_id: &str,
    artist: &str,
    title: &str,
) -> Option<String> {
    if !song_id.is_empty() {
        if let Ok(sub) = call_json(
            srv,
            "getLyricsBySongId",
            &format!("id={}", urlencoding::encode(song_id)),
        )
        .await
        {
            if let Some(list) = sub
                .pointer("/lyricsList/structuredLyrics")
                .and_then(|v| v.as_array())
            {
                if let Some(first) = list.first() {
                    if let Some(lines) = first.get("line").and_then(|v| v.as_array()) {
                        let mut out = String::new();
                        for l in lines {
                            let val = l.get("value").and_then(|v| v.as_str()).unwrap_or("");
                            if let Some(ms) = l.get("start").and_then(|v| v.as_i64()) {
                                let mins = ms / 60000;
                                let secs = (ms % 60000) / 1000;
                                let millis = ms % 1000;
                                out.push_str(&format!("[{mins:02}:{secs:02}.{millis:03}]{val}\n"));
                            } else {
                                out.push_str(val);
                                out.push('\n');
                            }
                        }
                        if !out.trim().is_empty() {
                            return Some(out);
                        }
                    }
                }
            }
        }
    }
    if let Ok(sub) = call_json(
        srv,
        "getLyrics",
        &format!(
            "artist={}&title={}",
            urlencoding::encode(artist),
            urlencoding::encode(title)
        ),
    )
    .await
    {
        if let Some(text) = sub.pointer("/lyrics/value").and_then(|v| v.as_str()) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}
