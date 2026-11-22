use base64::Engine;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Runtime};
use tauri_plugin_carminium_audio::{CarminiumAudioExt, HttpFetchRequest};

const DESKTOP_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent(DESKTOP_UA)
        .build()
        .map_err(|e| e.to_string())
}

pub async fn fetch_text_native<R: Runtime>(
    app: &AppHandle<R>,
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Result<(u16, String), String> {
    let mut header_map = HashMap::new();
    header_map.insert("User-Agent".to_string(), DESKTOP_UA.to_string());
    for (k, v) in headers {
        header_map.insert((*k).to_string(), (*v).to_string());
    }

    // Prefer native iOS URLSession via plugin so China-region iOS wireless-data permission
    // prompts, system proxies/VPNs, Keychain CAs, and gzip/br decoding work natively.
    if let Ok(resp) = app
        .carminium_audio()
        .http_fetch(HttpFetchRequest {
            url: url.to_string(),
            method: method.to_string(),
            headers: header_map.clone(),
            body: body.map(|s| s.to_string()),
            timeout_secs: Some(12.0),
            response_base64: Some(false),
        })
        .await
    {
        if resp.error.as_deref() != Some("desktop_unavailable") {
            if let Some(err) = resp.error {
                return Err(err);
            }
            return Ok((resp.status, resp.body));
        }
    }

    // Fallback to reqwest (desktop / non-iOS)
    let client = http_client()?;
    let mut req = match method {
        "POST" => client.post(url),
        _ => client.get(url),
    };
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    if let Some(b) = body {
        req = req.body(b.to_string());
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    Ok((status, text))
}

pub async fn fetch_bytes_native<R: Runtime>(
    app: &AppHandle<R>,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<Vec<u8>, String> {
    let mut header_map = HashMap::new();
    header_map.insert("User-Agent".to_string(), DESKTOP_UA.to_string());
    for (k, v) in headers {
        header_map.insert((*k).to_string(), (*v).to_string());
    }

    if let Ok(resp) = app
        .carminium_audio()
        .http_fetch(HttpFetchRequest {
            url: url.to_string(),
            method: "GET".to_string(),
            headers: header_map,
            body: None,
            timeout_secs: Some(12.0),
            response_base64: Some(true),
        })
        .await
    {
        if resp.error.as_deref() != Some("desktop_unavailable") {
            if let Some(err) = resp.error {
                return Err(err);
            }
            if !(200..300).contains(&resp.status) {
                return Err(format!("HTTP {}", resp.status));
            }
            return base64::engine::general_purpose::STANDARD
                .decode(resp.body.trim())
                .map_err(|e| format!("base64 decode error: {e}"));
        }
    }

    let client = http_client()?;
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| e.to_string())
}

async fn fetch_text<R: Runtime>(
    app: &AppHandle<R>,
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> Result<(u16, String), String> {
    fetch_text_native(app, method, url, headers, body).await
}

fn strip_jsonp(raw: &str) -> &str {
    let s = raw.trim();
    if let Some(rest) = s.strip_prefix("callback(") {
        let end = rest
            .strip_suffix(");")
            .or_else(|| rest.strip_suffix(')'))
            .unwrap_or(rest);
        return end.trim();
    }
    if let Some(rest) = s.strip_prefix("MusicJsonCallback(") {
        let end = rest
            .strip_suffix(");")
            .or_else(|| rest.strip_suffix(')'))
            .unwrap_or(rest);
        return end.trim();
    }
    s
}

pub async fn search_lyrics<R: Runtime>(app: &AppHandle<R>, query: &str, source: &str) -> Value {
    let q = query.trim();
    if q.is_empty() {
        return json!({ "songs": [], "error": null });
    }
    match source {
        "lrclib" => search_lrclib(app, q).await,
        "qqmusic" => search_qqmusic(app, q).await,
        "amll" => search_ncm(app, q).await,
        _ => search_ncm(app, q).await,
    }
}

pub async fn fetch_lyrics_by_id<R: Runtime>(
    app: &AppHandle<R>,
    song_id: &str,
    source: &str,
) -> Value {
    match source {
        "lrclib" => fetch_lrclib(app, song_id).await,
        "qqmusic" => fetch_qqmusic(app, song_id).await,
        "amll" => fetch_amll_or_ncm(app, song_id).await,
        _ => fetch_ncm(app, song_id).await,
    }
}

// ── NetEase Cloud Music ─────────────────────────────────────────────

async fn search_ncm<R: Runtime>(app: &AppHandle<R>, query: &str) -> Value {
    let form_body = format!("s={}&type=1&limit=20&offset=0", urlencoding::encode(query));
    let ncm_headers = [
        ("Referer", "https://music.163.com/"),
        ("Origin", "https://music.163.com"),
        ("Cookie", "os=pc; appver=2.9.7"),
        ("Content-Type", "application/x-www-form-urlencoded"),
    ];
    let endpoints = [
        "https://music.163.com/api/search/get",
        "https://music.163.com/api/cloudsearch/pc",
    ];

    let mut last_err: Option<String> = None;
    for ep in endpoints {
        match fetch_text(app, "POST", ep, &ncm_headers, Some(&form_body)).await {
            Ok((_, raw)) => {
                if let Ok(data) = serde_json::from_str::<Value>(&raw) {
                    let songs_arr = data
                        .pointer("/result/songs")
                        .and_then(|v| v.as_array());
                    if let Some(arr) = songs_arr {
                        if !arr.is_empty() {
                            let songs: Vec<Value> = arr
                                .iter()
                                .map(|s| {
                                    let artist_list = s
                                        .get("artists")
                                        .or_else(|| s.get("ar"))
                                        .and_then(|a| a.as_array());
                                    let artists = artist_list
                                        .map(|list| {
                                            list.iter()
                                                .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                                                .collect::<Vec<_>>()
                                                .join(", ")
                                        })
                                        .unwrap_or_default();
                                    let album = s
                                        .pointer("/album/name")
                                        .or_else(|| s.pointer("/al/name"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let duration = s
                                        .get("duration")
                                        .or_else(|| s.get("dt"))
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0);
                                    json!({
                                        "id": s.get("id").cloned().unwrap_or(Value::Null),
                                        "name": s.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                        "artist": artists,
                                        "album": album,
                                        "duration": duration,
                                    })
                                })
                                .collect();
                            return json!({ "songs": songs, "error": null });
                        }
                    }
                } else {
                    last_err = Some("invalid json from ncm".to_string());
                }
            }
            Err(e) => {
                last_err = Some(e);
            }
        }
    }
    json!({ "songs": [], "error": last_err })
}

fn yrc_to_enhanced_lrc(yrc: &str) -> String {
    let mut out = String::new();
    for raw_line in yrc.lines() {
        let line = raw_line.trim();
        if !line.starts_with('[') {
            continue;
        }
        let Some(end_bracket) = line.find(']') else {
            continue;
        };
        let header = &line[1..end_bracket];
        let mut parts = header.split(',');
        let Ok(line_start_ms) = parts.next().unwrap_or("").trim().parse::<i64>() else {
            continue;
        };
        let rest = &line[end_bracket + 1..];
        let mut buf = format_ts(line_start_ms);
        let mut cur = rest;
        let mut any_word = false;
        while let Some(open_p) = cur.find('(') {
            let after_open = &cur[open_p + 1..];
            let Some(close_p) = after_open.find(')') else {
                break;
            };
            let meta = &after_open[..close_p];
            let after_close = &after_open[close_p + 1..];
            let next_open = after_close.find('(').unwrap_or(after_close.len());
            let word = &after_close[..next_open];
            if let Some(w_start_str) = meta.split(',').next() {
                if let Ok(w_start) = w_start_str.trim().parse::<i64>() {
                    let mins = w_start / 60_000;
                    let secs = (w_start % 60_000) / 1000;
                    let millis = w_start % 1000;
                    buf.push_str(&format!("<{mins:02}:{secs:02}.{millis:03}>{word}"));
                    any_word = true;
                }
            }
            cur = &after_close[next_open..];
        }
        if any_word {
            out.push_str(&buf);
            out.push('\n');
        }
    }
    out
}

async fn fetch_ncm<R: Runtime>(app: &AppHandle<R>, song_id: &str) -> Value {
    let url = format!(
        "https://music.163.com/api/song/lyric?id={}&lv=1&kv=1&tv=-1&yv=-1",
        urlencoding::encode(song_id)
    );
    let raw = match fetch_text(
        app,
        "GET",
        &url,
        &[
            ("Referer", "https://music.163.com/"),
            ("Cookie", "os=pc; appver=2.9.7"),
        ],
        None,
    )
    .await
    {
        Ok((_, text)) => text,
        Err(e) => {
            return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": e })
        }
    };
    let data: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": format!("parse error: {e}") })
        }
    };
    let lrc = data
        .pointer("/lrc/lyric")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let tlyric = data
        .pointer("/tlyric/lyric")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let romalrc = data
        .pointer("/romalrc/lyric")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let yrclrc = data
        .pointer("/yrc/lyric")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let enhanced_yrc = if !yrclrc.trim().is_empty() {
        yrc_to_enhanced_lrc(yrclrc)
    } else {
        String::new()
    };
    let original = if !enhanced_yrc.trim().is_empty() {
        enhanced_yrc.as_str()
    } else {
        lrc
    };

    let combined = combine_lrc(original, romalrc, tlyric);
    json!({
        "lyrics": combined,
        "has_translation": !tlyric.trim().is_empty(),
        "has_romaji": !romalrc.trim().is_empty(),
        "error": null,
    })
}

async fn fetch_amll_or_ncm<R: Runtime>(app: &AppHandle<R>, song_id: &str) -> Value {
    // Try AMLL TTML DB GitHub mirror by NetEase ID first, fall back to NetEase LRC
    let url = format!(
        "https://raw.githubusercontent.com/Steve-xmh/amll-ttml-db/main/ncm-lyrics/{song_id}.ttml"
    );
    if let Ok((status, text)) = fetch_text(app, "GET", &url, &[], None).await {
        if (200..300).contains(&status) && !text.trim().is_empty() {
            return json!({
                "lyrics": text,
                "has_translation": text.contains("x-translation"),
                "has_romaji": text.contains("x-roman"),
                "error": null,
            });
        }
    }
    fetch_ncm(app, song_id).await
}

// ── LRCLIB ──────────────────────────────────────────────────────────

async fn search_lrclib<R: Runtime>(app: &AppHandle<R>, query: &str) -> Value {
    let url = format!("https://lrclib.net/api/search?q={}", urlencoding::encode(query));
    let raw = match fetch_text(app, "GET", &url, &[], None).await {
        Ok((_, text)) => text,
        Err(e) => return json!({ "songs": [], "error": e }),
    };
    let list: Vec<Value> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => return json!({ "songs": [], "error": format!("parse error: {e}") }),
    };
    let songs: Vec<Value> = list
        .into_iter()
        .take(20)
        .map(|s| {
            let dur_sec = s.get("duration").and_then(|v| v.as_f64()).unwrap_or(0.0);
            json!({
                "id": s.get("id").cloned().unwrap_or(Value::Null),
                "name": s.get("trackName").and_then(|v| v.as_str()).unwrap_or(""),
                "artist": s.get("artistName").and_then(|v| v.as_str()).unwrap_or(""),
                "album": s.get("albumName").and_then(|v| v.as_str()).unwrap_or(""),
                "duration": (dur_sec * 1000.0) as i64,
            })
        })
        .collect();
    json!({ "songs": songs, "error": null })
}

async fn fetch_lrclib<R: Runtime>(app: &AppHandle<R>, song_id: &str) -> Value {
    let url = format!("https://lrclib.net/api/get/{}", urlencoding::encode(song_id));
    let raw = match fetch_text(app, "GET", &url, &[], None).await {
        Ok((_, text)) => text,
        Err(e) => {
            return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": e })
        }
    };
    let data: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": format!("parse error: {e}") })
        }
    };
    let synced = data
        .get("syncedLyrics")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let plain = data
        .get("plainLyrics")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let lyrics = if !synced.trim().is_empty() {
        synced
    } else {
        plain
    };
    json!({
        "lyrics": lyrics,
        "has_translation": false,
        "has_romaji": false,
        "error": null,
    })
}

// ── QQ Music ────────────────────────────────────────────────────────

async fn search_qqmusic<R: Runtime>(app: &AppHandle<R>, query: &str) -> Value {
    let url = format!(
        "https://c.y.qq.com/soso/fcgi-bin/client_search_cp?w={}&p=1&n=20&format=json",
        urlencoding::encode(query)
    );
    let raw = match fetch_text(app, "GET", &url, &[("Referer", "https://y.qq.com/")], None).await {
        Ok((_, text)) => text,
        Err(e) => return json!({ "songs": [], "error": e }),
    };
    let data: Value = match serde_json::from_str(strip_jsonp(&raw)) {
        Ok(v) => v,
        Err(e) => return json!({ "songs": [], "error": format!("parse error: {e}") }),
    };
    let songs: Vec<Value> = data
        .pointer("/data/song/list")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|s| {
                    let artists = s
                        .get("singer")
                        .and_then(|a| a.as_array())
                        .map(|list| {
                            list.iter()
                                .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    json!({
                        "id": s.get("songmid").and_then(|v| v.as_str()).unwrap_or(""),
                        "name": s.get("songname").and_then(|v| v.as_str()).unwrap_or(""),
                        "artist": artists,
                        "album": s.get("albumname").and_then(|v| v.as_str()).unwrap_or(""),
                        "duration": s.get("interval").and_then(|v| v.as_i64()).unwrap_or(0) * 1000,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({ "songs": songs, "error": null })
}

async fn fetch_qqmusic<R: Runtime>(app: &AppHandle<R>, song_mid: &str) -> Value {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let url = format!(
        "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid={}&format=json&pcachetime={}",
        urlencoding::encode(song_mid),
        now_ms
    );
    let raw = match fetch_text(app, "GET", &url, &[("Referer", "https://y.qq.com/")], None).await {
        Ok((_, text)) => text,
        Err(e) => {
            return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": e })
        }
    };
    let data: Value = match serde_json::from_str(strip_jsonp(&raw)) {
        Ok(v) => v,
        Err(e) => {
            return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": format!("parse error: {e}") })
        }
    };
    let decode_b64 = |key: &str| -> String {
        data.get(key)
            .and_then(|v| v.as_str())
            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default()
    };
    let lrc = decode_b64("lyric");
    let trans = decode_b64("trans");
    let combined = combine_lrc(&lrc, "", &trans);
    json!({
        "lyrics": combined,
        "has_translation": !trans.trim().is_empty(),
        "has_romaji": false,
        "error": null,
    })
}

// ── LRC Merging Helper ──────────────────────────────────────────────

fn parse_lrc_timestamp_ms(tag: &str) -> Option<i64> {
    let parts: Vec<&str> = tag.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let mins: i64 = parts[0].trim().parse().ok()?;
    let sec_parts: Vec<&str> = parts[1].split(&['.', ':'][..]).collect();
    let secs: i64 = sec_parts[0].trim().parse().ok()?;
    let millis: i64 = if sec_parts.len() > 1 {
        let frac: String = sec_parts[1]
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .take(3)
            .collect();
        match frac.len() {
            0 => 0,
            1 => frac.parse::<i64>().ok()? * 100,
            2 => frac.parse::<i64>().ok()? * 10,
            _ => frac.parse::<i64>().ok()?,
        }
    } else {
        0
    };
    Some(mins * 60_000 + secs * 1000 + millis)
}

fn parse_lrc_map(raw: &str) -> BTreeMap<i64, Vec<String>> {
    let mut map: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('[') {
            continue;
        }
        let mut rest = trimmed;
        let mut timestamps = Vec::new();
        while rest.starts_with('[') {
            if let Some(end) = rest.find(']') {
                let inside = &rest[1..end];
                if let Some(ms) = parse_lrc_timestamp_ms(inside) {
                    timestamps.push(ms);
                    rest = &rest[end + 1..];
                    continue;
                }
            }
            break;
        }
        let text = rest.trim();
        if timestamps.is_empty() {
            continue;
        }
        for ms in timestamps {
            map.entry(ms).or_default().push(text.to_string());
        }
    }
    map
}

fn format_ts(ms: i64) -> String {
    let mins = ms / 60_000;
    let secs = (ms % 60_000) / 1000;
    let millis = ms % 1000;
    format!("[{mins:02}:{secs:02}.{millis:03}]")
}

fn combine_lrc(orig: &str, romaji: &str, trans: &str) -> String {
    if romaji.trim().is_empty() && trans.trim().is_empty() {
        return orig.to_string();
    }
    let orig_map = parse_lrc_map(orig);
    if orig_map.is_empty() {
        return orig.to_string();
    }
    let roma_map = parse_lrc_map(romaji);
    let trans_map = parse_lrc_map(trans);

    let mut out = String::new();
    for (ms, lines) in &orig_map {
        let ts = format_ts(*ms);
        for l in lines {
            out.push_str(&ts);
            out.push_str(l);
            out.push('\n');
        }
        if let Some(rlines) = roma_map.get(ms) {
            for r in rlines {
                if !r.is_empty() {
                    out.push_str(&ts);
                    out.push_str(r);
                    out.push('\n');
                }
            }
        }
        if let Some(tlines) = trans_map.get(ms) {
            for t in tlines {
                if !t.is_empty() {
                    out.push_str(&ts);
                    out.push_str(t);
                    out.push('\n');
                }
            }
        }
    }
    out
}

// ── Online Artist Image / Bio / Aliases (Native URLSession + Fallback) ──

async fn search_ncm_artist<R: Runtime>(app: &AppHandle<R>, artist_name: &str) -> Option<Value> {
    let name = artist_name.trim();
    if name.is_empty() {
        return None;
    }
    let body = format!(
        "s={}&type=100&offset=0&total=true&limit=6",
        urlencoding::encode(name)
    );
    let headers = [
        ("Referer", "https://music.163.com/"),
        ("Origin", "https://music.163.com"),
        ("Cookie", "os=pc; appver=2.9.7"),
        ("Content-Type", "application/x-www-form-urlencoded"),
    ];
    let (_, raw) = fetch_text(
        app,
        "POST",
        "https://music.163.com/api/search/get/web",
        &headers,
        Some(&body),
    )
    .await
    .ok()?;
    let data: Value = serde_json::from_str(&raw).ok()?;
    let list = data.pointer("/result/artists")?.as_array()?;
    let target = name.to_lowercase();
    let best = list
        .iter()
        .find(|a| {
            a.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_lowercase() == target)
                .unwrap_or(false)
        })
        .or_else(|| list.first())?;
    Some(best.clone())
}

pub async fn fetch_artist_image_online<R: Runtime>(
    app: &AppHandle<R>,
    artist_name: &str,
) -> Option<Vec<u8>> {
    let name = artist_name.trim();
    if name.is_empty() || name == "未知艺术家" || name.eq_ignore_ascii_case("unknown artist") {
        return None;
    }

    // 1. NetEase Cloud Music artist avatar
    if let Some(artist) = search_ncm_artist(app, name).await {
        let raw_url = artist
            .get("img1v1Url")
            .or_else(|| artist.get("picUrl"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !raw_url.is_empty() && !raw_url.contains("5639395138885805") {
            let sized = if raw_url.contains('?') {
                format!("{raw_url}&param=500y500")
            } else {
                format!("{raw_url}?param=500y500")
            };
            if let Ok(bytes) =
                fetch_bytes_native(app, &sized, &[("Referer", "https://music.163.com/")]).await
            {
                if bytes.len() > 256 {
                    return Some(bytes);
                }
            }
        }
    }

    // 2. Deezer artist avatar fallback
    let dz_url = format!(
        "https://api.deezer.com/search/artist/?q={}&limit=5",
        urlencoding::encode(name)
    );
    if let Ok((_, raw)) = fetch_text(app, "GET", &dz_url, &[], None).await {
        if let Ok(data) = serde_json::from_str::<Value>(&raw) {
            if let Some(list) = data.get("data").and_then(|v| v.as_array()) {
                let target = name.to_lowercase();
                let hit = list
                    .iter()
                    .find(|a| {
                        a.get("name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim().to_lowercase() == target)
                            .unwrap_or(false)
                    })
                    .or_else(|| list.first());
                if let Some(img_url) = hit.and_then(|a| {
                    a.get("picture_big")
                        .or_else(|| a.get("picture_xl"))
                        .and_then(|v| v.as_str())
                }) {
                    if let Ok(bytes) = fetch_bytes_native(app, img_url, &[]).await {
                        if bytes.len() > 256 {
                            return Some(bytes);
                        }
                    }
                }
            }
        }
    }

    None
}

pub async fn fetch_artist_bio_online<R: Runtime>(
    app: &AppHandle<R>,
    artist_name: &str,
) -> Option<String> {
    let artist = search_ncm_artist(app, artist_name).await?;
    let id = artist.get("id").and_then(|v| v.as_i64())?;
    let url = format!("https://music.163.com/api/artist/desc?id={id}");
    let (_, raw) = fetch_text(
        app,
        "GET",
        &url,
        &[
            ("Referer", "https://music.163.com/"),
            ("Cookie", "os=pc; appver=2.9.7"),
        ],
        None,
    )
    .await
    .ok()?;
    let data: Value = serde_json::from_str(&raw).ok()?;
    let desc = data
        .get("briefDesc")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    Some(desc)
}

pub async fn fetch_artist_aliases_online<R: Runtime>(
    app: &AppHandle<R>,
    artist_name: &str,
) -> Vec<String> {
    let Some(artist) = search_ncm_artist(app, artist_name).await else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    if let Some(arr) = artist.get("alias").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                let t = s.trim();
                if !t.is_empty() && !out.iter().any(|x| x == t) {
                    out.push(t.to_string());
                }
            }
        }
    }
    if let Some(tr) = artist.get("trans").and_then(|v| v.as_str()) {
        let t = tr.trim();
        if !t.is_empty() && !out.iter().any(|x| x == t) {
            out.push(t.to_string());
        }
    }
    out
}
