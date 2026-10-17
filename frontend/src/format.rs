pub fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

pub fn fmt_speed(bps: f64) -> String {
    if bps <= 0.0 {
        "—".to_string()
    } else {
        format!("{}/s", fmt_bytes(bps as u64))
    }
}

pub fn fmt_eta(secs: Option<u64>) -> String {
    let Some(s) = secs else { return String::new() };
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    }
}

/// RunTimeTicks → "1h 52m" / "42m" / "8m 30s"
pub fn fmt_runtime(ticks: Option<i64>) -> String {
    let Some(t) = ticks else { return String::new() };
    let total_min = t / 600_000_000;
    if total_min >= 60 {
        format!("{}h {}m", total_min / 60, total_min % 60)
    } else {
        format!("{total_min}m")
    }
}

/// Estimated final size for a transcode preset. Primary model: the source
/// codec is kept, so the file shrinks roughly with the pixel-count ratio.
/// Falls back to the fixed-ladder bitrate when source dimensions are unknown.
pub fn estimate_size(
    preset: &shared::QualityPreset,
    media: Option<&shared::MediaInfo>,
    ticks: Option<i64>,
) -> Option<u64> {
    let orig = media.and_then(|m| m.size)?;
    let src_dim = media.and_then(|m| m.width.map(|w| w.max(m.height.unwrap_or(0))));
    let target_dim = preset.max_width.or(preset.max_height)?;
    match src_dim {
        Some(s) if s > 0 => {
            let ratio = ((target_dim as f64 / s as f64).powi(2)).clamp(0.02, 1.0);
            Some((orig as f64 * ratio) as u64)
        }
        _ => {
            let secs = ticks? as f64 / 10_000_000.0;
            let bps = preset.estimate_bps?;
            Some((bps as f64 * secs / 8.0) as u64)
        }
    }
}
