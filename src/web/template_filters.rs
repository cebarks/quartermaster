#[allow(dead_code)] // Called by Askama filter functions via generated code
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
}
