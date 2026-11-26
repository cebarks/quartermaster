//! HTTP routes for the immutable Convoy v2 publisher.
use actix_web::{
    body::SizedStream,
    http::header::{self, Header, IfNoneMatch},
    web, HttpRequest, HttpResponse,
};
use futures_util::stream;
use std::os::unix::fs::FileExt;

use crate::convoy::publisher::{Admission, DeliveryError, ObjectLease, Publisher};

pub fn configure_v2(cfg: &mut web::ServiceConfig) {
    cfg.route("/convoy/v2/catalog", web::get().to(catalog))
        .route(
            "/convoy/v2/generations/{generation}/objects/{sha256}",
            web::get().to(object),
        );
}

fn error(err: DeliveryError) -> HttpResponse {
    let (mut response, code) = match err {
        DeliveryError::Preparing => (HttpResponse::ServiceUnavailable(), "preparing"),
        DeliveryError::Busy => (HttpResponse::ServiceUnavailable(), "busy"),
        DeliveryError::Disabled => (HttpResponse::ServiceUnavailable(), "disabled"),
        DeliveryError::PublishFailed => (HttpResponse::ServiceUnavailable(), "publish_failed"),
        DeliveryError::StaleGeneration => (HttpResponse::Conflict(), "stale_generation"),
        DeliveryError::NotPublished => (HttpResponse::NotFound(), "not_published"),
    };
    if matches!(err, DeliveryError::Busy | DeliveryError::Preparing) {
        response.insert_header((header::RETRY_AFTER, "1"));
    }
    response
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(serde_json::json!({"code": code}))
}

async fn catalog(publisher: web::Data<Publisher>, req: HttpRequest) -> HttpResponse {
    let (generation, json) = match publisher.admit(None, None).await {
        Ok(Admission::Manifest { generation, json }) => (generation, json),
        Ok(_) => return error(DeliveryError::PublishFailed),
        Err(err) => return error(err),
    };
    let etag = header::EntityTag::new_strong(generation);
    let matched = match IfNoneMatch::parse(&req) {
        Ok(IfNoneMatch::Any) => true,
        Ok(IfNoneMatch::Items(tags)) => tags.iter().any(|tag| tag.weak_eq(&etag)),
        Err(_) => false,
    };
    let mut response = if matched {
        HttpResponse::NotModified()
    } else {
        HttpResponse::Ok()
    };
    response
        .insert_header((header::ETAG, etag))
        .insert_header((header::CACHE_CONTROL, "no-cache"));
    if matched {
        response.finish()
    } else {
        response.content_type("application/json").body(json)
    }
}

// No Range means full. Malformed or unsupported syntax is rejected even if the
// request could not resume; a valid Range without a matching validator is ignored.
fn range_start(req: &HttpRequest, etag: &str, length: u64) -> Result<Option<u64>, ()> {
    let mut ranges = req.headers().get_all(header::RANGE);
    let Some(range) = ranges.next() else {
        return Ok(None);
    };
    if ranges.next().is_some() {
        return Err(());
    }
    let digits = range
        .to_str()
        .ok()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.strip_suffix('-'))
        .ok_or(())?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    let mut validators = req.headers().get_all(header::IF_RANGE);
    if !validators.next().is_some_and(|v| v == etag) || validators.next().is_some() {
        return Ok(None);
    }
    let start: u64 = digits.parse().map_err(|_| ())?;
    if start >= length {
        return Err(());
    }
    Ok(Some(start))
}

async fn object(
    publisher: web::Data<Publisher>,
    req: HttpRequest,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (generation, digest) = path.into_inner();
    let (file, length, lease) = match publisher.admit(Some(&generation), Some(&digest)).await {
        Ok(Admission::Object {
            file,
            length,
            lease,
        }) => (file, length, lease),
        Ok(_) => return error(DeliveryError::PublishFailed),
        Err(err) => return error(err),
    };
    object_response(&req, &digest, file, length, Some(lease))
}

fn object_response(
    req: &HttpRequest,
    digest: &str,
    file: std::fs::File,
    length: u64,
    lease: Option<ObjectLease>,
) -> HttpResponse {
    let etag = format!("\"{digest}\"");
    let start = match range_start(req, &etag, length) {
        Ok(start) => start,
        Err(()) => {
            return HttpResponse::RangeNotSatisfiable()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .insert_header((header::ACCEPT_RANGES, "bytes"))
                .insert_header((header::CONTENT_RANGE, format!("bytes */{length}")))
                .insert_header((header::ETAG, etag))
                .body(web::Bytes::new())
        }
    };
    let mut response = if let Some(start) = start {
        let mut partial = HttpResponse::PartialContent();
        partial.insert_header((
            header::CONTENT_RANGE,
            format!("bytes {start}-{}/{length}", length - 1),
        ));
        partial
    } else {
        HttpResponse::Ok()
    };
    let offset = start.unwrap_or(0);
    let remaining = length - offset;
    let exact = stream::try_unfold(
        (file, lease, offset),
        move |(file, lease, offset)| async move {
            if offset == length {
                return Ok::<_, std::io::Error>(None);
            }
            // The file and its pin stay together even if the HTTP body is dropped
            // while the blocking read is already running.
            tokio::task::spawn_blocking(move || {
                let mut chunk = vec![0; (length - offset).min(64 * 1024) as usize];
                file.read_exact_at(&mut chunk, offset)?;
                let next = offset + chunk.len() as u64;
                Ok(Some((web::Bytes::from(chunk), (file, lease, next))))
            })
            .await
            .map_err(std::io::Error::other)?
        },
    );
    response
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .insert_header((header::ACCEPT_RANGES, "bytes"))
        .insert_header((header::ETAG, etag))
        .content_type("application/octet-stream")
        .body(SizedStream::new(remaining, exact))
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{App, HttpServer};

    #[actix_web::test]
    async fn shortened_open_object_fails_http_body() {
        let fixture = tempfile::NamedTempFile::new().expect("create fixture");
        std::fs::write(fixture.path(), b"abc").expect("write fixture bytes");
        let path = fixture.path().to_owned();
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind disposable loopback socket");
        let endpoint = format!(
            "http://{}/object",
            listener.local_addr().expect("read bound loopback address")
        );
        let server = HttpServer::new(move || {
            let path = path.clone();
            App::new().route(
                "/object",
                web::get().to(move |req: HttpRequest| {
                    let path = path.clone();
                    async move {
                        let admitted =
                            std::fs::File::open(&path).expect("open fixture file or database");
                        let length = admitted.metadata().expect("inspect fixture file").len();
                        std::fs::OpenOptions::new()
                            .write(true)
                            .open(path)
                            .expect("fixture operation must succeed")
                            .set_len(1)
                            .expect("fixture operation must succeed");
                        object_response(
                            &req,
                            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                            admitted,
                            length,
                            None,
                        )
                    }
                }),
            )
        })
        .workers(1)
        .listen(listener)
        .expect("fixture operation must succeed")
        .run();
        let handle = server.handle();
        actix_web::rt::spawn(server);
        let response = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .expect("fixture operation must succeed")
            .get(endpoint)
            .send()
            .await
            .expect("fixture operation must succeed");
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(response.headers()["content-length"], "3");
        let failure = response
            .bytes()
            .await
            .expect_err("truncated object body must fail");
        assert!(failure.is_body() || failure.is_decode(), "{failure:?}");
        assert!(!failure.is_timeout(), "{failure:?}");
        handle.stop(false).await;
    }
}
