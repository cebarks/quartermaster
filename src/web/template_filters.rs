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
pub fn fmt_pct(val: &f64, _env: &dyn askama::Values) -> askama::Result<String> {
    Ok(format!("{:.1}%", val))
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

fn format_roubles_value(n: i64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

#[askama::filter_fn]
pub fn format_roubles(value: &Option<i64>, _env: &dyn askama::Values) -> askama::Result<String> {
    match value {
        Some(n) => Ok(format!("₽{}", format_roubles_value(*n))),
        None => Ok("—".to_string()),
    }
}

#[askama::filter_fn]
pub fn format_roubles_i64(value: &i64, _env: &dyn askama::Values) -> askama::Result<String> {
    Ok(format!("₽{}", format_roubles_value(*value)))
}

/// Truncate a datetime string to just the date+time portion (first 19 chars: `YYYY-MM-DD HH:MM:SS`).
#[askama::filter_fn]
pub fn format_datetime(s: &str, _env: &dyn askama::Values) -> askama::Result<String> {
    Ok(s.chars().take(19).collect())
}

// ponytail: askama invokes via generated code, clippy can't see usage
#[allow(dead_code)]
#[askama::filter_fn]
pub fn markdown(content: &str, _env: &dyn askama::Values) -> askama::Result<String> {
    use pulldown_cmark::{html, Options, Parser};
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(content, opts);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    Ok(ammonia::clean(&html_output))
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
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

    #[test]
    fn format_roubles_thousands() {
        assert_eq!(format_roubles_value(1500), "1,500");
    }

    #[test]
    fn format_roubles_millions() {
        assert_eq!(format_roubles_value(6_722_609), "6,722,609");
    }

    #[test]
    fn format_roubles_zero() {
        assert_eq!(format_roubles_value(0), "0");
    }

    #[test]
    fn format_roubles_small() {
        assert_eq!(format_roubles_value(999), "999");
    }

    #[test]
    fn format_roubles_exact_thousand() {
        assert_eq!(format_roubles_value(1000), "1,000");
    }

    fn render_markdown(content: &str) -> String {
        use pulldown_cmark::{html, Options, Parser};
        let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
        let parser = Parser::new_ext(content, opts);
        let mut html_output = String::new();
        html::push_html(&mut html_output, parser);
        ammonia::clean(&html_output)
    }

    #[test]
    fn markdown_renders_basic() {
        let html = render_markdown("**bold**");
        assert!(html.contains("<strong>bold</strong>"));
    }

    #[test]
    fn markdown_sanitizes_scripts() {
        let html = render_markdown("<script>alert(1)</script>");
        assert!(!html.contains("<script>"));
    }

    #[test]
    fn markdown_renders_strikethrough() {
        let html = render_markdown("~~deleted~~");
        assert!(html.contains("<del>deleted</del>"));
    }
}
