use crate::forge::client::ForgeClient;
use crate::forge::models::FikaCompat;

pub(crate) async fn open_named_file(
    path: std::path::PathBuf,
) -> std::io::Result<actix_files::NamedFile> {
    actix_web::web::block(move || actix_files::NamedFile::open(path))
        .await
        .map_err(std::io::Error::other)?
}

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
    let url = reqwest::Url::parse(input).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(url.host_str(), Some("sp-mod.com" | "forge.sp-tarkov.com"))
    {
        return None;
    }
    let mut segments = url.path_segments()?;
    let route = segments.next()?;
    let id_segment = segments.next()?;
    match route {
        "mod" => id_segment.parse().ok(),
        "mods" => id_segment.split('-').next()?.parse().ok(),
        _ => None,
    }
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
    use actix_web::body::to_bytes;
    use actix_web::http::{header, StatusCode};
    use actix_web::test::TestRequest;
    use std::path::PathBuf;

    fn archive_fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.zip");
        std::fs::write(&path, b"0123456789").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))
            .unwrap();
        (dir, path)
    }

    #[actix_web::test]
    async fn named_file_serves_attachment() {
        let (_dir, path) = archive_fixture();
        let full = open_named_file(path)
            .await
            .unwrap()
            .set_content_disposition(header::ContentDisposition {
                disposition: header::DispositionType::Attachment,
                parameters: vec![header::DispositionParam::Filename("mods.zip".to_string())],
            })
            .into_response(&TestRequest::get().to_http_request());
        assert_eq!(full.status(), StatusCode::OK);
        assert_eq!(
            full.headers()
                .get(header::CONTENT_DISPOSITION)
                .unwrap()
                .to_str()
                .unwrap(),
            "attachment; filename=\"mods.zip\""
        );
        assert_eq!(
            &to_bytes(full.into_body()).await.unwrap()[..],
            b"0123456789"
        );
    }

    #[actix_web::test]
    async fn named_file_range_response_is_partial() {
        let (_dir, path) = archive_fixture();
        let request = TestRequest::get()
            .insert_header((header::RANGE, "bytes=2-4"))
            .to_http_request();
        let range = open_named_file(path).await.unwrap().into_response(&request);
        assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            range.headers().get(header::CONTENT_RANGE).unwrap(),
            "bytes 2-4/10"
        );
        assert_eq!(&to_bytes(range.into_body()).await.unwrap()[..], b"234");
    }

    #[actix_web::test]
    async fn named_file_etag_revalidation_is_not_modified() {
        let (_dir, path) = archive_fixture();
        let full = open_named_file(path.clone())
            .await
            .unwrap()
            .into_response(&TestRequest::get().to_http_request());
        let etag = full.headers().get(header::ETAG).unwrap().clone();
        let request = TestRequest::get()
            .insert_header((header::IF_NONE_MATCH, etag))
            .to_http_request();
        let cached = open_named_file(path).await.unwrap().into_response(&request);
        assert_eq!(cached.status(), StatusCode::NOT_MODIFIED);
        assert!(to_bytes(cached.into_body()).await.unwrap().is_empty());
    }

    #[actix_web::test]
    async fn named_file_modified_since_revalidation_is_not_modified() {
        let (_dir, path) = archive_fixture();
        let full = open_named_file(path.clone())
            .await
            .unwrap()
            .into_response(&TestRequest::get().to_http_request());
        let modified = full.headers().get(header::LAST_MODIFIED).unwrap().clone();
        let request = TestRequest::get()
            .insert_header((header::IF_MODIFIED_SINCE, modified))
            .to_http_request();
        let since = open_named_file(path).await.unwrap().into_response(&request);
        assert_eq!(since.status(), StatusCode::NOT_MODIFIED);
        assert!(to_bytes(since.into_body()).await.unwrap().is_empty());
    }

    #[actix_web::test]
    async fn named_file_missing_path_preserves_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let error = open_named_file(dir.path().join("missing.zip"))
            .await
            .err()
            .expect("missing file must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }

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
    fn parse_forge_url_current_and_legacy_mod_links() {
        for url in [
            "https://sp-mod.com/mod/2326/project-fika",
            "https://sp-mod.com/mod/2326/project-fika/?details=true#versions",
            "https://sp-mod.com/mods/2326-project-fika",
            "https://forge.sp-tarkov.com/mod/2326/project-fika",
        ] {
            assert_eq!(parse_forge_url(url), Some(2326), "{url}");
        }
    }

    #[test]
    fn parse_forge_url_rejects_unrelated_hosts_and_paths() {
        for url in [
            "https://sp-mod.com.example.org/mod/2326/project-fika",
            "https://example.org/mods/2326-sp-mod.com",
            "https://forge.sp-tarkov.com.example.org/mods/2326-test",
            "https://sp-mod.com/addon/2326/test",
            "https://sp-mod.com/mod/not-an-id/project-fika",
        ] {
            assert_eq!(parse_forge_url(url), None, "{url}");
        }
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
