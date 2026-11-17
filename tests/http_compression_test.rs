use actix_web::{http::header, middleware::Compress, test, web, App, HttpResponse};

#[actix_web::test]
async fn http_compression_retains_gzip_brotli_but_not_zstd() {
    let app = test::init_service(App::new().wrap(Compress::default()).route(
        "/",
        web::get().to(|| async {
            HttpResponse::Ok()
                .content_type("text/plain")
                .body("quartermaster codec regression ".repeat(256))
        }),
    ))
    .await;
    for encoding in ["gzip", "br"] {
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri("/")
                .insert_header((header::ACCEPT_ENCODING, encoding))
                .to_request(),
        )
        .await;
        assert!(response.status().is_success());
        assert_eq!(
            response.headers().get(header::CONTENT_ENCODING).unwrap(),
            encoding
        );
        assert!(!test::read_body(response).await.is_empty());
    }
    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/")
            .insert_header((header::ACCEPT_ENCODING, "zstd"))
            .to_request(),
    )
    .await;
    assert!(
        response.status().is_success()
            || response.status() == actix_web::http::StatusCode::NOT_ACCEPTABLE
    );
    assert_ne!(
        response
            .headers()
            .get(header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok()),
        Some("zstd")
    );
}

#[actix_web::test]
async fn compressed_payload_limits_remain_enforced() {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;

    const LIMIT: usize = 64 * 1024;
    const BR_8: &[u8] = &[226, 0, 0, 191, 68, 88, 136, 16, 0, 0];
    const BR_65537: &[u8] = &[10, 0, 32, 240, 75, 132, 197, 99, 129, 128, 238, 61, 0];
    let app = test::init_service(App::new().app_data(web::PayloadConfig::new(LIMIT)).route(
        "/echo",
        web::post().to(|body: web::Bytes| async move { HttpResponse::Ok().body(body) }),
    ))
    .await;
    for decoded_size in [8, LIMIT + 1] {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&vec![b'a'; decoded_size]).unwrap();
        let gzip = encoder.finish().unwrap();
        let brotli = if decoded_size == 8 { BR_8 } else { BR_65537 };
        for (encoding, encoded) in [("gzip", gzip.as_slice()), ("br", brotli)] {
            assert!(encoded.len() < LIMIT);
            let response = test::call_service(
                &app,
                test::TestRequest::post()
                    .uri("/echo")
                    .insert_header((header::CONTENT_ENCODING, encoding))
                    .set_payload(encoded.to_vec())
                    .to_request(),
            )
            .await;
            if decoded_size == 8 {
                assert_eq!(
                    response.status(),
                    actix_web::http::StatusCode::OK,
                    "{encoding}"
                );
                assert_eq!(
                    &test::read_body(response).await[..],
                    b"aaaaaaaa",
                    "{encoding}"
                );
            } else {
                assert_eq!(
                    response.status(),
                    actix_web::http::StatusCode::PAYLOAD_TOO_LARGE,
                    "{encoding}"
                );
            }
        }
    }
}
