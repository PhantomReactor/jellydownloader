//! A tiny fake Jellyfin server used to test the download pipeline end to end.
//! Generates real media with ffmpeg at startup so both download paths
//! (original + HLS transcode) can be exercised.
//!
//! Run:  cargo run -p backend --bin mock-jellyfin

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde_json::json;
use std::sync::Arc;

struct MockState {
    original: Bytes,
    segs: Vec<Bytes>,
    playlist: Arc<String>,
    init: Bytes,
    /// When the "transcode" started; segment i becomes available 1.5s after this.
    started: std::time::Instant,
}

const MEDIA_DIR: &str = "/tmp/opencode/mockmedia";
const PORT: u16 = 8095;

fn generate_media() {
    std::fs::remove_dir_all(MEDIA_DIR).ok();
    std::fs::create_dir_all(MEDIA_DIR).unwrap();
    let run = |args: &[&str]| {
        let status = std::process::Command::new("ffmpeg")
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("ffmpeg must be installed for the mock server");
        assert!(status.success(), "ffmpeg failed for args {args:?}");
    };
    run(&[
        "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24:duration=30",
        "-f", "lavfi", "-i", "sine=frequency=440:duration=30",
        "-c:v", "libx264", "-preset", "veryfast", "-crf", "30", "-pix_fmt", "yuv420p",
        "-c:a", "aac", "-b:a", "64k", "-movflags", "+faststart",
        &format!("{MEDIA_DIR}/original.mp4"),
    ]);
    for i in 0..10u32 {
        run(&[
            "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24:duration=3",
            "-f", "lavfi", "-i", &format!("sine=frequency={}:duration=3", 300 + i * 40),
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "30", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-b:a", "64k", "-f", "mpegts",
            &format!("{MEDIA_DIR}/seg_{i:02}.ts"),
        ]);
    }
}

#[tokio::main]
async fn main() {
    generate_media();
    let original = std::fs::read(format!("{MEDIA_DIR}/original.mp4")).unwrap();
    let mut segs = Vec::new();
    for i in 0..10u32 {
        segs.push(Bytes::from(std::fs::read(format!("{MEDIA_DIR}/seg_{i:02}.ts")).unwrap()));
    }
    // fMP4 style: init segment + ENDLIST up front (like Jellyfin's VOD playlists),
    // while segment bytes are muxed lazily by the "encoder" (see lazy handler).
    let mut pl = String::from(
        "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:3\n#EXT-X-MEDIA-SEQUENCE:0\n",
    );
    pl.push_str("#EXT-X-MAP:URI=\"m3u8/mock/init.mp4\"\n");
    for i in 0..10u32 {
        pl.push_str(&format!("#EXTINF:3.000000,\nm3u8/mock/seg_{i:02}.m4s\n"));
    }
    pl.push_str("#EXT-X-ENDLIST\n");

    // A minimal (invalid-codec but structurally fine) init segment: concat the
    // test segments" moov boxes is overkill; ffmpeg accepts whatever if we
    // actually feed valid fmp4. Use a tiny valid fmp4 init: generate one.
    let init_status = std::process::Command::new("ffmpeg")
        .args([
            "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24:duration=0.04",
            "-f", "lavfi", "-i", "sine=frequency=440:duration=0.04",
            "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-movflags", "+frag_keyframe+empty_moov",
            "-f", "mp4", &format!("{MEDIA_DIR}/init_ref.mp4"),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let init = if init_status {
        Bytes::from(std::fs::read(format!("{MEDIA_DIR}/init_ref.mp4")).unwrap())
    } else {
        Bytes::new()
    };

    let state = Arc::new(MockState {
        original: Bytes::from(original),
        segs,
        playlist: Arc::new(pl),
        init,
        started: std::time::Instant::now(),
    });

    let app = Router::new()
        .route("/Users/AuthenticateByName", post(auth))
        .route("/Users/{user_id}/Views", get(views))
        .route("/Users/{user_id}/Items", get(items))
        .route("/Users/{user_id}/Items/{id}", get(item_detail))
        .route("/Items/{id}/Images/Primary", get(poster))
        .route("/Items/{id}/Images/Backdrop", get(poster))
        .route("/Shows/{id}/Episodes", get(episodes))
        .route("/Videos/{id}/stream", get(stream))
        .route("/Videos/{id}/master.m3u8", get(master_m3u8))
        .route("/Videos/{id}/m3u8/mock/{seg}", get(segment))
        .with_state(state);

    let bind = std::env::var("MJ_BIND").unwrap_or_else(|_| "127.0.0.1".to_string());
    let addr = format!("{bind}:{PORT}");
    println!("mock jellyfin listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn auth() -> impl IntoResponse {
    Json(json!({
        "AccessToken": "mocktoken123",
        "User": { "Id": "mock-user-uid", "Name": "Mock User" }
    }))
}

async fn views() -> impl IntoResponse {
    Json(json!({
        "Items": [
            { "Id": "lib-movies", "Name": "Movies", "Type": "CollectionFolder", "CollectionType": "movies" },
            { "Id": "lib-tv", "Name": "TV Shows", "Type": "CollectionFolder", "CollectionType": "tvshows" }
        ]
    }))
}

fn base_items() -> serde_json::Value {
    json!({
        "Items": [
            { "Id": "m1", "Name": "Interstellar Mock", "Type": "Movie", "ProductionYear": 2014, "RunTimeTicks": 300_000_000 },
            { "Id": "m2", "Name": "Arrival Mock", "Type": "Movie", "ProductionYear": 2016, "RunTimeTicks": 300_000_000 },
            { "Id": "s1", "Name": "Mock Show", "Type": "Series", "ProductionYear": 2020 }
        ]
    })
}

async fn items(axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>) -> impl IntoResponse {
    let mut resp = base_items();
    // ParentId scoping: movies / tv libraries contain only their own kind
    if let Some(pid) = q.get("ParentId") {
        let keep: Option<&str> = match pid.as_str() {
            "lib-movies" => Some("Movie"),
            "lib-tv" => Some("Series"),
            _ => None,
        };
        if let Some(ty) = keep {
            let items = resp["Items"].as_array().cloned().unwrap_or_default();
            let filtered: Vec<_> = items.into_iter().filter(|i| i["Type"] == *ty).collect();
            resp["Items"] = json!(filtered);
        }
    }
    if let Some(term) = q.get("SearchTerm").filter(|t| !t.is_empty()) {
        let lower = term.to_lowercase();
        let items = resp["Items"].as_array().cloned().unwrap_or_default();
        let filtered: Vec<_> = items
            .into_iter()
            .filter(|i| i["Name"].as_str().map(|n| n.to_lowercase().contains(&lower)).unwrap_or(false))
            .collect();
        resp["Items"] = json!(filtered);
    }
    Json(resp)
}

fn movie_detail(id: &str) -> serde_json::Value {
    json!({
        "Id": id,
        "Name": if id == "m1" { "Interstellar Mock" } else { "Arrival Mock" },
        "Type": "Movie",
        "ProductionYear": if id == "m1" { 2014 } else { 2016 },
        "Overview": "A mock movie served by the fake Jellyfin server.",
        "RunTimeTicks": 300_000_000,
        "MediaSources": [{
            "Id": "ms-m1",
            "Container": "mp4",
            "Size": std::fs::metadata(format!("{MEDIA_DIR}/original.mp4")).map(|m| m.len()).unwrap_or(0),
            "MediaStreams": [
                { "Type": "Video", "Codec": "hevc", "Width": 1920, "Height": 800, "BitRate": 5_700_000 },
                { "Type": "Audio", "Codec": "aac", "Channels": 2 }
            ]
        }]
    })
}

fn episode_detail(id: &str, num: i64) -> serde_json::Value {
    json!({
        "Id": id,
        "Name": format!("Episode {num}"),
        "Type": "Episode",
        "SeriesName": "Mock Show",
        "ParentIndexNumber": 1,
        "IndexNumber": num,
        "ProductionYear": 2020,
        "RunTimeTicks": 300_000_000,
        "SeriesId": "s1",
        "MediaSources": [{
            "Id": "ms-m1",
            "Container": "mp4",
            "Size": std::fs::metadata(format!("{MEDIA_DIR}/original.mp4")).map(|m| m.len()).unwrap_or(0),
            "MediaStreams": [
                { "Type": "Video", "Codec": "hevc", "Width": 1920, "Height": 800, "BitRate": 5_700_000 },
                { "Type": "Audio", "Codec": "aac", "Channels": 2 }
            ]
        }]
    })
}

async fn item_detail(Path((_user_id, id)): Path<(String, String)>) -> Response {
    match id.as_str() {
        "m1" | "m2" => Json(movie_detail(&id)).into_response(),
        "s1" => Json(json!({
            "Id": "s1", "Name": "Mock Show", "Type": "Series", "ProductionYear": 2020,
            "Overview": "A mock series with two episodes."
        }))
        .into_response(),
        "e1" => Json(episode_detail("e1", 1)).into_response(),
        "e2" => Json(episode_detail("e2", 2)).into_response(),
        _ => (StatusCode::NOT_FOUND, Json(json!({"message": "not found"}))).into_response(),
    }
}

async fn poster(Path(id): Path<String>) -> Response {
    // tiny 2x2 PNG so the image proxy test has real bytes to move
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
        0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02,
        0x08, 0x02, 0x00, 0x00, 0x00, 0xFD, 0xD4, 0x9A, 0x73, 0x00, 0x00, 0x00,
        0x1D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xFF, 0xFF, 0x3F,
        0x03, 0x22, 0x5F, 0x01, 0x00, 0x1F, 0x5B, 0x05, 0x12, 0x38, 0xFE, 0x85,
        0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    let _ = id;
    Response::builder()
        .header(header::CONTENT_TYPE, "image/png")
        .body(Body::from(PNG))
        .unwrap()
}

async fn episodes(Path(id): Path<String>) -> impl IntoResponse {
    if id == "s1" {
        Json(json!({
            "Items": [
                { "Id": "e1", "Name": "Pilot", "Type": "Episode", "SeriesName": "Mock Show", "ParentIndexNumber": 1, "IndexNumber": 1, "ProductionYear": 2020, "RunTimeTicks": 300_000_000 },
                { "Id": "e2", "Name": "Second", "Type": "Episode", "SeriesName": "Mock Show", "ParentIndexNumber": 1, "IndexNumber": 2, "ProductionYear": 2020, "RunTimeTicks": 300_000_000 }
            ]
        }))
    } else {
        Json(json!({ "Items": [] }))
    }
}

fn parse_range(range: Option<&str>, total: u64) -> Option<(u64, u64)> {
    let range = range?;
    let spec = range.strip_prefix("bytes=")?;
    let (start_s, end_s) = spec.split_once('-')?;
    if start_s.is_empty() {
        let suffix: u64 = end_s.parse().ok()?;
        let start = total.saturating_sub(suffix);
        return Some((start, total.saturating_sub(1)));
    }
    let start: u64 = start_s.parse().ok()?;
    if start >= total {
        return None;
    }
    let end = if end_s.is_empty() { total - 1 } else { end_s.parse::<u64>().ok()?.min(total - 1) };
    Some((start, end))
}

async fn stream(State(st): State<Arc<MockState>>, headers: HeaderMap) -> Response {
    serve_bytes(st.original.clone(), headers).await
}

fn throttled_body(data: Bytes, chunk: usize, delay: std::time::Duration) -> Body {
    Body::from_stream(futures::stream::unfold(
        (data, 0usize),
        move |(d, pos)| async move {
            if pos >= d.len() {
                return None;
            }
            tokio::time::sleep(delay).await;
            let end = (pos + chunk).min(d.len());
            let c = d.slice(pos..end);
            Some((Ok::<_, std::io::Error>(c), (d, end)))
        },
    ))
}

async fn serve_bytes(data: Bytes, headers: HeaderMap) -> Response {
    let total = data.len() as u64;
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    match parse_range(range, total) {
        Some((start, end)) => {
            let slice = data.slice(start as usize..(end + 1) as usize);
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"))
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CONTENT_LENGTH, (end - start + 1).to_string())
                .body(throttled_body(slice, 16_384, std::time::Duration::from_millis(30)))
                .unwrap()
        }
        None => Response::builder()
            .status(StatusCode::OK)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_LENGTH, total.to_string())
            .body(throttled_body(data, 16_384, std::time::Duration::from_millis(30)))
            .unwrap(),
    }
}

async fn master_m3u8(State(st): State<Arc<MockState>>) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
        .body(Body::from((*st.playlist).clone()))
        .unwrap()
}

async fn segment(State(st): State<Arc<MockState>>, Path((_id, seg)): Path<(String, String)>) -> Response {
    if seg == "init.mp4" {
        // "encoder" needs 2s before the init segment exists
        let waited = st.started.elapsed() > std::time::Duration::from_secs(2)
            || st.started.elapsed() < std::time::Duration::from_millis(50);
        if !waited {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        return Response::builder()
            .header(header::CONTENT_TYPE, "video/mp4")
            .body(Body::from(st.init.clone()))
            .unwrap();
    }
    // lazy fMP4 segment: succeed only once its "encode turn" passed
    if let Some(idx) = seg.strip_prefix("seg_").and_then(|r| r.trim_end_matches(".m4s").parse::<usize>().ok()) {
        if (idx as usize) < st.segs.len() {
            let available_at = std::time::Duration::from_millis(1500 * (idx as u64 + 1));
            let since = st.started.elapsed();
            if available_at > since {
                tokio::time::sleep(available_at - since).await;
            }
            return Response::builder()
                .header(header::CONTENT_TYPE, "video/iso.segment")
                .body(Body::from(st.segs[idx].clone()))
                .unwrap();
        }
    }
    // legacy TS route
    let idx: Option<u32> = seg.trim_start_matches("seg_").trim_end_matches(".ts").parse().ok();
    match idx.filter(|i| (*i as usize) < st.segs.len()) {
        Some(i) => Response::builder()
            .header(header::CONTENT_TYPE, "video/mp2t")
            .body(Body::from(st.segs[i as usize].clone()))
            .unwrap(),
        None => Response::builder().status(StatusCode::NOT_FOUND).body(Body::empty()).unwrap(),
    }
}
