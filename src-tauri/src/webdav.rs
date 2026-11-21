use crate::db::{md5_hex, now_secs, LibraryDb, Track, WebDavServer};
use base64::Engine;
use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::Method;
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::time::Duration;

const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "m4a", "aac", "wav", "alac", "aif", "aiff", "ogg", "opus", "caf", "mp4",
];

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(20))
        .user_agent("ProjectCarminium/iOS")
        .build()
        .map_err(|e| format!("http client init failed: {e}"))
}

pub fn basic_auth_header(username: &str, password: &str) -> String {
    let raw = format!("{username}:{password}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(raw.as_bytes());
    format!("Basic {encoded}")
}

pub async fn test_server(srv: &WebDavServer) -> Result<Value, String> {
    let client = http_client()?;
    let method = Method::from_bytes(b"PROPFIND").map_err(|e| e.to_string())?;
    let mut req = client
        .request(method, &srv.server_url)
        .header("Depth", "0");
    if !srv.username.is_empty() || !srv.password.is_empty() {
        req = req.header("Authorization", basic_auth_header(&srv.username, &srv.password));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("连接 WebDAV 失败: {e}"))?;
    let status = resp.status().as_u16();
    if status == 200 || status == 207 {
        Ok(json!({ "ok": true, "status": status }))
    } else if status == 401 || status == 403 {
        Err("认证失败（用户名或密码错误）".to_string())
    } else {
        Err(format!("WebDAV 返回 HTTP {status}"))
    }
}

struct DavEntry {
    href: String,
    is_collection: bool,
    size: i64,
}

fn parse_propfind_xml(xml: &str) -> Vec<DavEntry> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut entries = Vec::new();
    let mut in_response = false;
    let mut current_tag = String::new();
    let mut href = String::new();
    let mut is_collection = false;
    let mut size: i64 = 0;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_lowercase();
                if local == "response" {
                    in_response = true;
                    href.clear();
                    is_collection = false;
                    size = 0;
                } else if in_response {
                    if local == "collection" {
                        is_collection = true;
                    }
                    current_tag = local;
                }
            }
            Ok(Event::Empty(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_lowercase();
                if in_response && local == "collection" {
                    is_collection = true;
                }
            }
            Ok(Event::Text(e)) => {
                if in_response {
                    if let Ok(txt) = e.unescape() {
                        match current_tag.as_str() {
                            "href" => href.push_str(txt.trim()),
                            "getcontentlength" => {
                                if let Ok(n) = txt.trim().parse::<i64>() {
                                    size = n;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_lowercase();
                if local == "response" && in_response {
                    if !href.is_empty() {
                        entries.push(DavEntry {
                            href: href.clone(),
                            is_collection,
                            size,
                        });
                    }
                    in_response = false;
                }
                current_tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    entries
}

fn resolve_dav_href(base_url: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    if let Some(scheme_end) = base_url.find("://") {
        let rest = &base_url[scheme_end + 3..];
        let host_end = rest.find('/').map(|i| scheme_end + 3 + i).unwrap_or(base_url.len());
        let origin = &base_url[..host_end];
        if href.starts_with('/') {
            return format!("{origin}{href}");
        }
        return format!("{}/{}", base_url.trim_end_matches('/'), href);
    }
    href.to_string()
}

pub async fn sync_server(db: &LibraryDb, srv: &WebDavServer) -> Result<Value, String> {
    let client = http_client()?;
    let method = Method::from_bytes(b"PROPFIND").map_err(|e| e.to_string())?;
    let auth = if !srv.username.is_empty() || !srv.password.is_empty() {
        Some(basic_auth_header(&srv.username, &srv.password))
    } else {
        None
    };

    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();
    let root = srv.server_url.trim_end_matches('/').to_string();
    visited.insert(root.clone());
    queue.push_back((root.clone(), 0));

    let now = now_secs();
    let mut tracks: Vec<Track> = Vec::new();

    while let Some((dir_url, depth)) = queue.pop_front() {
        if depth > 6 {
            continue;
        }
        let mut req = client
            .request(method.clone(), &dir_url)
            .header("Depth", "1");
        if let Some(ref a) = auth {
            req = req.header("Authorization", a);
        }
        let Ok(resp) = req.send().await else {
            continue;
        };
        if !resp.status().is_success() && resp.status().as_u16() != 207 {
            continue;
        }
        let Ok(xml) = resp.text().await else {
            continue;
        };

        let entries = parse_propfind_xml(&xml);
        let dir_norm = dir_url.trim_end_matches('/');

        for entry in entries {
            let full = resolve_dav_href(&dir_url, &entry.href);
            let full_norm = full.trim_end_matches('/').to_string();
            if full_norm == dir_norm {
                continue;
            }
            if entry.is_collection || entry.href.ends_with('/') {
                if visited.insert(full_norm.clone()) {
                    queue.push_back((full_norm, depth + 1));
                }
                continue;
            }

            let decoded = urlencoding::decode(&full)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| full.clone());
            let file_name = decoded.rsplit('/').next().unwrap_or("Unknown");
            let ext = file_name
                .rsplit('.')
                .next()
                .map(|s| s.to_lowercase())
                .unwrap_or_default();
            if !AUDIO_EXTS.contains(&ext.as_str()) {
                continue;
            }

            let stem = file_name
                .strip_suffix(&format!(".{ext}"))
                .unwrap_or(file_name)
                .to_string();
            let folder_name = decoded
                .rsplit('/')
                .nth(1)
                .unwrap_or("WebDAV")
                .to_string();
            let tid = format!("webdav:{}:{}", srv.id, md5_hex(&full));
            let sort_key = stem.trim().to_lowercase();

            tracks.push(Track {
                id: tid,
                path: full,
                folder_id: None,
                title: stem,
                artist: String::new(),
                album: folder_name,
                album_artist: String::new(),
                track_number: None,
                disc_number: None,
                year: None,
                duration_ms: 0,
                file_size: entry.size,
                has_cover: 0,
                lyrics: None,
                alias_title: None,
                added_at: now,
                source: "webdav".to_string(),
                server_id: Some(srv.id),
                subsonic_id: None,
                cover_id: None,
                suffix: Some(ext),
                genre: None,
                sort_key,
            });
        }
    }

    let count = db
        .replace_remote_tracks("webdav", srv.id, &tracks)
        .map_err(|e| format!("保存 WebDAV 曲目失败: {e}"))?;
    db.touch_webdav_sync(srv.id);

    Ok(json!({
        "tracks": count,
        "added": count,
    }))
}
