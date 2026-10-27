mod common;

use actix_web::{http::StatusCode, test::TestRequest};
use common::TestAppBuilder;

const PROFILE: &str = "000000000000000000000001";

#[actix_web::test]
async fn only_approved_prestige_artwork_gets_bypass_authentication() {
    let mut app = TestAppBuilder::new().with_proxy_auth().build().await;
    for path in [
        "/files/prestige/0123456789abcdef01234567.png",
        "/files/prestige/reward/small/0123456789abcdef01234567.png",
        "/files/prestige/reward/big/0123456789abcdef01234567.png",
    ] {
        assert_eq!(app.get(path).await.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            app.send_raw(
                TestRequest::get()
                    .uri(path)
                    .append_header(("authorization", "Basic first"))
                    .append_header(("authorization", "Basic second"))
                    .to_request()
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            app.send_raw(TestRequest::post().uri(path).to_request())
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        app.get("/files/user/profiles/0123456789abcdef01234567.json")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[actix_web::test]
async fn headless_websocket_credentials_never_use_ordinary_player_auth_as_a_shortcut() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let mut app = TestAppBuilder::new()
        .with_proxy_auth()
        .with_user("player", "test-password", "player")
        .build()
        .await;
    let user_id = {
        let db = app.db.lock();
        let id = db.get_user_by_username("player").unwrap().unwrap().id;
        db.update_user_spt_profile_id(id, Some(PROFILE)).unwrap();
        id
    };
    let request = || {
        TestRequest::get()
            .uri("/fika/headless/client")
            .peer_addr("127.0.0.1:12345".parse().unwrap())
            .insert_header(("upgrade", "websocket"))
            .insert_header(("connection", "Upgrade"))
            .insert_header(("sec-websocket-version", "13"))
            .insert_header(("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="))
    };
    let basic = format!("Basic {}", STANDARD.encode(format!("{PROFILE}:")));
    assert_eq!(
        app.send_raw(
            request()
                .insert_header(("authorization", basic.clone()))
                .to_request()
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send_raw(
            request()
                .insert_header(("cookie", format!("PHPSESSID={PROFILE}")))
                .to_request()
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.send_raw(
            request()
                .peer_addr("203.0.113.1:12345".parse().unwrap())
                .insert_header(("authorization", basic.clone()))
                .insert_header(("cookie", format!("PHPSESSID={PROFILE}")))
                .to_request()
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    for (first, second) in [
        (PROFILE, "000000000000000000000002"),
        ("000000000000000000000002", PROFILE),
        (PROFILE, PROFILE),
    ] {
        let expected = if first == second {
            StatusCode::BAD_GATEWAY
        } else {
            StatusCode::UNAUTHORIZED
        };
        assert_eq!(
            app.send_raw(
                TestRequest::get()
                    .uri("/client/game/start")
                    .append_header(("cookie", format!("PHPSESSID={first}")))
                    .append_header(("cookie", format!("PHPSESSID={second}")))
                    .to_request()
            )
            .await
            .status(),
            expected
        );
        assert_eq!(
            app.send_raw(
                TestRequest::get()
                    .uri("/client/game/start")
                    .insert_header(("cookie", format!("PHPSESSID={first}; phpsessid={second}")))
                    .to_request()
            )
            .await
            .status(),
            expected
        );
    }
    app.db.lock().set_user_disabled(user_id, true).unwrap();
    assert_eq!(
        app.send_raw(
            request()
                .insert_header(("authorization", basic))
                .to_request()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
}

#[actix_web::test]
async fn bootstrap_get_is_forwarded_but_other_methods_and_routes_require_session() {
    let mut app = TestAppBuilder::new().with_proxy_auth().build().await;
    // 502 demonstrates that authentication allowed forwarding to the intentionally
    // unreachable upstream; this test must not talk to an actual SPT instance.
    let response = app.get("/singleplayer/customEnumEntries").await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    for path in [
        "/singleplayer/customEnumEntries/",
        "/singleplayer/customEnumEntries/extra",
        "/client/game/start",
    ] {
        assert_eq!(app.get(path).await.status(), StatusCode::UNAUTHORIZED);
    }
    let request = TestRequest::post()
        .uri("/singleplayer/customEnumEntries")
        .to_request();
    assert_eq!(
        app.send_raw(request).await.status(),
        StatusCode::UNAUTHORIZED
    );
}

#[actix_web::test]
async fn unknown_profile_is_denied_even_from_loopback_or_spoofed_forwarding_headers() {
    let mut app = TestAppBuilder::new().with_proxy_auth().build().await;
    for (peer, forwarded) in [("127.0.0.1:12345", false), ("203.0.113.1:12345", true)] {
        let mut request = TestRequest::get()
            .uri("/client/game/start")
            .peer_addr(peer.parse().unwrap())
            .insert_header(("cookie", format!("PHPSESSID={PROFILE}")));
        if forwarded {
            request = request.insert_header(("x-forwarded-for", "127.0.0.1"));
        }
        assert_eq!(
            app.send_raw(request.to_request()).await.status(),
            StatusCode::UNAUTHORIZED
        );
    }
}

#[actix_web::test]
async fn linked_player_still_requires_enabled_account() {
    let mut app = TestAppBuilder::new()
        .with_proxy_auth()
        .with_user("player", "test-password", "player")
        .build()
        .await;
    let user_id = {
        let db = app.db.lock();
        let id = db.get_user_by_username("player").unwrap().unwrap().id;
        db.update_user_spt_profile_id(id, Some(PROFILE)).unwrap();
        id
    };
    let request = || {
        TestRequest::get()
            .uri("/client/game/start")
            .peer_addr("127.0.0.1:12345".parse().unwrap())
            .insert_header(("cookie", format!("PHPSESSID={PROFILE}")))
            .to_request()
    };
    assert_eq!(
        app.send_raw(request()).await.status(),
        StatusCode::BAD_GATEWAY
    );
    app.db.lock().set_user_disabled(user_id, true).unwrap();
    assert_eq!(
        app.send_raw(request()).await.status(),
        StatusCode::FORBIDDEN
    );
}
