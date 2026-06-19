use anyhow::{bail, Result};
use rand::distr::Alphanumeric;
use rand::Rng;

pub fn generate_invite_code() -> String {
    let suffix: String = rand::rng()
        .sample_iter(Alphanumeric)
        .take(10)
        .map(char::from)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    format!("quma-{suffix}")
}

pub fn parse_expiry(input: &str) -> Result<String> {
    let input = input.trim();
    let (num_str, unit) = if let Some(stripped) = input.strip_suffix('d') {
        (stripped, "days")
    } else if let Some(stripped) = input.strip_suffix('h') {
        (stripped, "hours")
    } else if let Some(stripped) = input.strip_suffix('m') {
        (stripped, "minutes")
    } else {
        bail!("invalid expiry format: use e.g. '24h', '7d', '30m'");
    };

    let num: i64 = num_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid number in expiry: '{num_str}'"))?;

    if num <= 0 {
        bail!("expiry must be positive");
    }

    let duration = match unit {
        "days" => chrono::Duration::days(num),
        "hours" => chrono::Duration::hours(num),
        "minutes" => chrono::Duration::minutes(num),
        _ => unreachable!(),
    };

    let expires_at = chrono::Utc::now() + duration;
    Ok(expires_at.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_code_format() {
        let code = generate_invite_code();
        assert!(code.starts_with("quma-"), "code should start with 'quma-'");
        assert_eq!(code.len(), 15, "code should be 15 chars: 'quma-' + 10");
        let suffix = &code[5..];
        assert!(
            suffix.chars().all(|c| c.is_ascii_alphanumeric()),
            "suffix should be alphanumeric"
        );
    }

    #[test]
    fn parse_expiry_hours() {
        let result = parse_expiry("24h").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result)
            .unwrap()
            .with_timezone(&chrono::Utc);
        let diff = parsed - chrono::Utc::now();
        assert!(diff.num_hours() >= 23 && diff.num_hours() <= 24);
    }

    #[test]
    fn parse_expiry_days() {
        let result = parse_expiry("7d").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result)
            .unwrap()
            .with_timezone(&chrono::Utc);
        let diff = parsed - chrono::Utc::now();
        assert!(diff.num_days() >= 6 && diff.num_days() <= 7);
    }

    #[test]
    fn parse_expiry_minutes() {
        let result = parse_expiry("30m").unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(&result)
            .unwrap()
            .with_timezone(&chrono::Utc);
        let diff = parsed - chrono::Utc::now();
        assert!(diff.num_minutes() >= 29 && diff.num_minutes() <= 30);
    }

    #[test]
    fn parse_expiry_invalid() {
        assert!(parse_expiry("abc").is_err());
        assert!(parse_expiry("0h").is_err());
        assert!(parse_expiry("-5d").is_err());
    }
}
