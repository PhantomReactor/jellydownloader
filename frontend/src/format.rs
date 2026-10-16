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

/// Estimated final size for a transcode preset, from item duration.
pub fn estimate_size(estimate_bps: Option<u64>, ticks: Option<i64>) -> Option<u64> {
    let secs = ticks? as f64 / 10_000_000.0;
    let bps = estimate_bps?;
    Some((bps as f64 * secs / 8.0) as u64)
}
