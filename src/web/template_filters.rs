fn format_bytes(n: i64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    if n == 0 {
        return "0 B".to_string();
    }
    let n = n as f64;
    let i = (n.log2() / 10.0).floor() as usize;
    let i = i.min(UNITS.len() - 1);
    let val = n / (1u64 << (i * 10)) as f64;
    if i == 0 {
        format!("{} B", val as i64)
    } else {
        format!("{:.1} {}", val, UNITS[i])
    }
}

#[askama::filter_fn]
pub fn format_size(bytes: &Option<i64>, _env: &dyn askama::Values) -> askama::Result<String> {
    match bytes {
        Some(n) => Ok(format_bytes(*n)),
        None => Ok("-".to_string()),
    }
}

#[askama::filter_fn]
pub fn format_size_i64(bytes: &i64, _env: &dyn askama::Values) -> askama::Result<String> {
    Ok(format_bytes(*bytes))
}

#[askama::filter_fn]
pub fn format_size_u64(bytes: &u64, _env: &dyn askama::Values) -> askama::Result<String> {
    Ok(format_bytes(*bytes as i64))
}

#[askama::filter_fn]
pub fn fmt_pct(val: &f64, _env: &dyn askama::Values) -> askama::Result<String> {
    Ok(format!("{:.1}%", val))
}

#[askama::filter_fn]
pub fn clamp_pct(val: &f64, _env: &dyn askama::Values) -> askama::Result<String> {
    Ok(format!("{:.1}", val.clamp(0.0, 100.0)))
}

fn compute_uptime(started_at: &str) -> Result<String, askama::Error> {
    use chrono::{DateTime, Utc};
    let started: DateTime<Utc> = started_at
        .parse()
        .map_err(|e| askama::Error::Custom(Box::new(e)))?;
    let duration = Utc::now() - started;
    let total_secs = duration.num_seconds().max(0);

    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;

    if days > 0 {
        Ok(format!("{}d {}h", days, hours))
    } else if hours > 0 {
        Ok(format!("{}h {}m", hours, minutes))
    } else if minutes > 0 {
        Ok(format!("{}m", minutes))
    } else {
        Ok("<1m".to_string())
    }
}

#[askama::filter_fn]
pub fn format_uptime(started_at: &str, _env: &dyn askama::Values) -> askama::Result<String> {
    compute_uptime(started_at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_zero() {
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn format_bytes_small() {
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn format_bytes_kib() {
        assert_eq!(format_bytes(1536), "1.5 KiB");
    }

    #[test]
    fn format_bytes_mib() {
        assert_eq!(format_bytes(2_621_440), "2.5 MiB");
    }

    #[test]
    fn format_bytes_gib() {
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
    }

    #[test]
    fn format_bytes_exact_kib() {
        assert_eq!(format_bytes(1024), "1.0 KiB");
    }

    #[test]
    fn uptime_minutes() {
        let now = chrono::Utc::now();
        let started = (now - chrono::Duration::minutes(45)).to_rfc3339();
        let result = compute_uptime(&started).unwrap();
        assert_eq!(result, "45m");
    }

    #[test]
    fn uptime_hours_and_minutes() {
        let now = chrono::Utc::now();
        let started =
            (now - chrono::Duration::hours(2) - chrono::Duration::minutes(15)).to_rfc3339();
        let result = compute_uptime(&started).unwrap();
        assert_eq!(result, "2h 15m");
    }

    #[test]
    fn uptime_days() {
        let now = chrono::Utc::now();
        let started = (now - chrono::Duration::days(3) - chrono::Duration::hours(4)).to_rfc3339();
        let result = compute_uptime(&started).unwrap();
        assert_eq!(result, "3d 4h");
    }

    #[test]
    fn uptime_just_started() {
        let now = chrono::Utc::now();
        let started = (now - chrono::Duration::seconds(30)).to_rfc3339();
        let result = compute_uptime(&started).unwrap();
        assert_eq!(result, "<1m");
    }

    #[test]
    fn uptime_invalid_timestamp() {
        let result = compute_uptime("not-a-timestamp");
        assert!(result.is_err());
    }
}
