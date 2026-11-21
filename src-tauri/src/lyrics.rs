use base64::Engine;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) \
             AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
        )
        .build()
        .map_err(|e| e.to_string())
}

pub async fn search_lyrics(query: &str, source: &str) -> Value {
    let q = query.trim();
    if q.is_empty() {
        return json!({ "songs": [], "error": null });
    }
    match source {
        "lrclib" => search_lrclib(q).await,
        "qqmusic" => search_qqmusic(q).await,
        "amll" => search_ncm(q).await,
        _ => search_ncm(q).await,
    }
}

pub async fn fetch_lyrics_by_id(song_id: &str, source: &str) -> Value {
    match source {
        "lrclib" => fetch_lrclib(song_id).await,
        "qqmusic" => fetch_qqmusic(song_id).await,
        "amll" => fetch_amll_or_ncm(song_id).await,
        _ => fetch_ncm(song_id).await,
    }
}

// ── NetEase Cloud Music ─────────────────────────────────────────────

async fn search_ncm(query: &str) -> Value {
    let Ok(client) = http_client() else {
        return json!({ "songs": [], "error": "http init failed" });
    };
    let res = client
        .post("https://music.163.com/api/search/get")
        .header("Referer", "https://music.163.com")
        .form(&[("s", query), ("type", "1"), ("limit", "20"), ("offset", "0")])
        .send()
        .await;
    let Ok(resp) = res else {
        return json!({ "songs": [], "error": "network error" });
    };
    let Ok(data) = resp.json::<Value>().await else {
        return json!({ "songs": [], "error": "invalid json" });
    };
    let songs: Vec<Value> = data
        .pointer("/result/songs")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|s| {
                    let artists = s
                        .get("artists")
                        .and_then(|a| a.as_array())
                        .map(|list| {
                            list.iter()
                                .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    json!({
                        "id": s.get("id").cloned().unwrap_or(Value::Null),
                        "name": s.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        "artist": artists,
                        "album": s.pointer("/album/name").and_then(|v| v.as_str()).unwrap_or(""),
                        "duration": s.get("duration").and_then(|v| v.as_i64()).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({ "songs": songs, "error": null })
}

async fn fetch_ncm(song_id: &str) -> Value {
    let Ok(client) = http_client() else {
        return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": "http init failed" });
    };
    let url = format!(
        "https://music.163.com/api/song/lyric?id={}&lv=1&kv=1&tv=-1&yv=-1",
        urlencoding::encode(song_id)
    );
    let Ok(resp) = client
        .get(&url)
        .header("Referer", "https://music.163.com")
        .send()
        .await
    else {
        return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": "network error" });
    };
    let Ok(data) = resp.json::<Value>().await else {
        return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": "invalid json" });
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

    let combined = combine_lrc(lrc, romalrc, tlyric);
    json!({
        "lyrics": combined,
        "has_translation": !tlyric.trim().is_empty(),
        "has_romaji": !romalrc.trim().is_empty(),
        "error": null,
    })
}

async fn fetch_amll_or_ncm(song_id: &str) -> Value {
    // Try AMLL TTML DB GitHub mirror by NetEase ID first, fall back to NetEase LRC
    if let Ok(client) = http_client() {
        let url = format!(
            "https://raw.githubusercontent.com/Steve-xmh/amll-ttml-db/main/ncm-lyrics/{song_id}.ttml"
        );
        if let Ok(resp) = client.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(text) = resp.text().await {
                    if !text.trim().is_empty() {
                        return json!({
                            "lyrics": text,
                            "has_translation": text.contains("x-translation"),
                            "has_romaji": text.contains("x-roman"),
                            "error": null,
                        });
                    }
                }
            }
        }
    }
    fetch_ncm(song_id).await
}

// ── LRCLIB ──────────────────────────────────────────────────────────

async fn search_lrclib(query: &str) -> Value {
    let Ok(client) = http_client() else {
        return json!({ "songs": [], "error": "http init failed" });
    };
    let url = format!("https://lrclib.net/api/search?q={}", urlencoding::encode(query));
    let Ok(resp) = client.get(&url).send().await else {
        return json!({ "songs": [], "error": "network error" });
    };
    let Ok(list) = resp.json::<Vec<Value>>().await else {
        return json!({ "songs": [], "error": "invalid json" });
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

async fn fetch_lrclib(song_id: &str) -> Value {
    let Ok(client) = http_client() else {
        return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": "http init failed" });
    };
    let url = format!("https://lrclib.net/api/get/{}", urlencoding::encode(song_id));
    let Ok(resp) = client.get(&url).send().await else {
        return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": "network error" });
    };
    let Ok(data) = resp.json::<Value>().await else {
        return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": "invalid json" });
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

async fn search_qqmusic(query: &str) -> Value {
    let Ok(client) = http_client() else {
        return json!({ "songs": [], "error": "http init failed" });
    };
    let url = format!(
        "https://c.y.qq.com/soso/fcgi-bin/client_search_cp?w={}&p=1&n=20&format=json",
        urlencoding::encode(query)
    );
    let Ok(resp) = client
        .get(&url)
        .header("Referer", "https://y.qq.com/")
        .send()
        .await
    else {
        return json!({ "songs": [], "error": "network error" });
    };
    let Ok(data) = resp.json::<Value>().await else {
        return json!({ "songs": [], "error": "invalid json" });
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

async fn fetch_qqmusic(song_mid: &str) -> Value {
    let Ok(client) = http_client() else {
        return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": "http init failed" });
    };
    let url = format!(
        "https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid={}&format=json&nobase64=0",
        urlencoding::encode(song_mid)
    );
    let Ok(resp) = client
        .get(&url)
        .header("Referer", "https://y.qq.com/")
        .send()
        .await
    else {
        return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": "network error" });
    };
    let Ok(data) = resp.json::<Value>().await else {
        return json!({ "lyrics": "", "has_translation": false, "has_romaji": false, "error": "invalid json" });
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
