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
