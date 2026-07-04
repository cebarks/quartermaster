use crate::forge::client::ForgeClient;
use crate::forge::models::FikaCompat;

pub struct ForgeSearchResult {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub fika_compatible: String,
}

pub(crate) fn parse_forge_url(input: &str) -> Option<i64> {
    let input = input.trim();
    if let Ok(id) = input.parse::<i64>() {
        return Some(id);
    }
    if input.contains("forge.sp-tarkov.com") {
        // Strip query parameters before parsing
        let url_path = input.split('?').next().unwrap_or(input);
        let parts: Vec<&str> = url_path.split('/').collect();
        if let Some(segment) = parts.iter().rev().find(|s| !s.is_empty()) {
            if let Some(id_str) = segment.split('-').next() {
                if let Ok(id) = id_str.parse::<i64>() {
                    return Some(id);
                }
            }
        }
    }
    None
}

pub(crate) fn fika_compat_to_string(fc: &Option<FikaCompat>) -> String {
    match fc {
        Some(FikaCompat::Compatible) => "compatible".to_string(),
        Some(FikaCompat::Incompatible) => "incompatible".to_string(),
        _ => "unknown".to_string(),
    }
}

pub async fn forge_search(
    forge: &ForgeClient,
    query: &str,
) -> (Vec<ForgeSearchResult>, Option<String>) {
    let q = query.trim();

    // Check for direct ID or URL first — single-digit mod IDs are valid
    if let Some(mod_id) = parse_forge_url(q) {
        return match forge.get_mod(mod_id, false).await {
            Ok(m) => (
                vec![ForgeSearchResult {
                    id: m.id,
                    name: m.name,
                    description: m.description,
                    fika_compatible: fika_compat_to_string(&m.fika_compatibility),
                }],
                None,
            ),
            Err(_) => (
                vec![],
                Some(format!("Mod with ID {mod_id} not found on Forge.")),
            ),
        };
    }

    // Only apply min-length guard for text searches (not numeric IDs/URLs handled above)
    if q.len() < 2 {
        return (vec![], None);
    }

    match forge.search_mods(q).await {
        Ok(mods) => {
            let results = mods
                .into_iter()
                .map(|m| ForgeSearchResult {
                    id: m.id,
                    name: m.name,
                    description: m.description,
                    fika_compatible: fika_compat_to_string(&m.fika_compatibility),
                })
                .collect();
            (results, None)
        }
        Err(_) => (
            vec![],
            Some("Could not reach SPT Forge. Try again later.".to_string()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_forge_url_numeric_id() {
        assert_eq!(parse_forge_url("2326"), Some(2326));
    }

    #[test]
    fn parse_forge_url_full_url() {
        assert_eq!(
            parse_forge_url("https://forge.sp-tarkov.com/mods/2326-some-mod"),
            Some(2326)
        );
    }

    #[test]
    fn parse_forge_url_url_with_trailing_slash() {
        assert_eq!(
            parse_forge_url("https://forge.sp-tarkov.com/mods/123-test/"),
            Some(123)
        );
    }

    #[test]
    fn parse_forge_url_plain_text() {
        assert_eq!(parse_forge_url("SAIN"), None);
    }

    #[test]
    fn parse_forge_url_empty() {
        assert_eq!(parse_forge_url(""), None);
    }

    #[test]
    fn parse_forge_url_whitespace() {
        assert_eq!(parse_forge_url("  2326  "), Some(2326));
    }

    #[test]
    fn parse_forge_url_with_query_params() {
        assert_eq!(
            parse_forge_url("https://forge.sp-tarkov.com/mods/2326-some-mod?details=true"),
            Some(2326)
        );
    }

    #[test]
    fn fika_compat_string_values() {
        use crate::forge::models::FikaCompat;
        assert_eq!(
            fika_compat_to_string(&Some(FikaCompat::Compatible)),
            "compatible"
        );
        assert_eq!(
            fika_compat_to_string(&Some(FikaCompat::Incompatible)),
            "incompatible"
        );
        assert_eq!(fika_compat_to_string(&Some(FikaCompat::Unknown)), "unknown");
        assert_eq!(fika_compat_to_string(&None), "unknown");
    }
}
