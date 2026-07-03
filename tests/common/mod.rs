use std::sync::Arc;

use actix_web::cookie::Key;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::test::{self, TestRequest};
use actix_web::{middleware, web, App};
use parking_lot::Mutex;
use tempfile::TempDir;

use spt_quartermaster::config::Config;
use spt_quartermaster::db::Database;
use spt_quartermaster::forge::client::ForgeClient;
use spt_quartermaster::spt::detect::SptInfo;
use spt_quartermaster::spt::game_data::GameData;
use spt_quartermaster::web::state::AppState;
use spt_quartermaster::web::{configure_app, proxy_metrics::ProxyMetrics};

/// Test app builder for integration tests.
pub struct TestAppBuilder {
    users: Vec<(String, String, String)>,
    mods: Vec<(i64, String, String)>,
    invites: Vec<(String, Option<String>)>, // (code, expires_at)
    external_url: Option<String>,
    mock_server: Option<wiremock::MockServer>,
}

impl TestAppBuilder {
    pub fn new() -> Self {
        Self {
            users: Vec::new(),
            mods: Vec::new(),
            invites: Vec::new(),
            external_url: None,
            mock_server: None,
        }
    }

    /// Seed a user into the test database.
    pub fn with_user(mut self, username: &str, password: &str, role: &str) -> Self {
        self.users
            .push((username.to_string(), password.to_string(), role.to_string()));
        self
    }

    /// Seed an installed mod into the test database.
    pub fn with_mod(mut self, forge_id: i64, name: &str, version: &str) -> Self {
        self.mods
            .push((forge_id, name.to_string(), version.to_string()));
        self
    }

    /// Seed an invite code into the test database.
    pub fn with_invite(mut self, code: &str, expires_at: Option<&str>) -> Self {
        self.invites
            .push((code.to_string(), expires_at.map(String::from)));
        self
    }

    /// Set the external_url in the test config.
    pub fn with_external_url(mut self, url: &str) -> Self {
        self.external_url = Some(url.to_string());
        self
    }

    /// Use a pre-configured mock server (instead of creating a new one).
    pub fn with_mock_server(mut self, server: wiremock::MockServer) -> Self {
        self.mock_server = Some(server);
        self
    }

    /// Build the test app.
    pub async fn build(self) -> TestApp {
        // Create a temporary SPT directory structure
        let tmp_dir = TempDir::new().expect("failed to create temp dir");
        let spt_dir = tmp_dir.path().to_path_buf();

        // SPT directory structure
        std::fs::create_dir_all(spt_dir.join("SPT")).unwrap();
        std::fs::write(spt_dir.join("SPT/SPT.Server.exe"), b"").unwrap();
        std::fs::create_dir_all(spt_dir.join("SPT/SPT_Data/configs")).unwrap();
        std::fs::write(
            spt_dir.join("SPT/SPT_Data/configs/core.json"),
            r#"{"sptVersion":"3.10.0","compatibleTarkovVersion":"0.16.0.xxxxx"}"#,
        )
        .unwrap();
        std::fs::write(spt_dir.join("SPT/SPT.Server.deps.json"), "{}").unwrap();
        std::fs::create_dir_all(spt_dir.join("SPT/user/mods")).unwrap();
        std::fs::create_dir_all(spt_dir.join("SPT/user/profiles")).unwrap();
        std::fs::create_dir_all(spt_dir.join("BepInEx/plugins")).unwrap();

        // Create an in-memory database
        let db = Database::open_in_memory().expect("failed to create in-memory DB");

        // Seed users
        for (username, password, role) in &self.users {
            let hashed = spt_quartermaster::web::auth::hash_password(password)
                .expect("failed to hash password");
            db.conn()
                .execute(
                    "INSERT INTO users (username, password_hash, role, disabled) VALUES (?, ?, ?, 0)",
                    rusqlite::params![username, hashed, role],
                )
                .expect("failed to insert user");
        }

        // Seed mods using the DB API
        for (forge_id, name, version) in &self.mods {
            db.insert_mod(*forge_id, 1, name, None, version)
                .expect("failed to insert mod");
        }

        // Seed invites
        for (code, expires_at) in &self.invites {
            db.create_invite(code, None, expires_at.as_deref())
                .expect("failed to insert invite");
        }

        // Start or reuse mock server
        let mock_server = match self.mock_server {
            Some(s) => s,
            None => wiremock::MockServer::start().await,
        };

        // Create ForgeClient pointing at mock server
        let forge = ForgeClient::with_base_url(mock_server.uri(), None)
            .expect("failed to create ForgeClient");

        // Build a test config with known session secret
        let config = Config {
            session_secret: "test-session-secret-at-least-48-chars-long-abcdefgh".to_string(),
            tls_enabled: false,
            proxy_enabled: false,
            external_url: self.external_url.clone(),
            ..Config::default()
        };

        // Write config file so handlers that reload config from disk can find it
        let config_path = spt_dir.join("quartermaster.toml");
        std::fs::write(
            &config_path,
            toml::to_string(&config).expect("failed to serialize config"),
        )
        .expect("failed to write config file");

        let session_key = Key::derive_from(config.session_secret.as_bytes());

        let (events_tx, _) =
            tokio::sync::broadcast::channel::<spt_quartermaster::web::sse::ServerEvent>(64);

        let proxy_client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(std::time::Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("failed to build proxy HTTP client");

        // Build AppState
        let db_arc = Arc::new(Mutex::new(db));
        let app_state = web::Data::new(AppState {
            db: db_arc.clone(),
            forge,
            config: Arc::new(parking_lot::RwLock::new(config.clone())),
            config_path,
            config_lock: parking_lot::Mutex::new(()),
            spt_dir: spt_dir.clone(),
            spt_info: SptInfo {
                root: spt_dir.clone(),
                spt_version: "3.10.0".to_string(),
                tarkov_version: "0.16.0.xxxxx".to_string(),
            },
            tasks: spt_quartermaster::web::tasks::TaskTracker::new(events_tx.clone()),
            update_cache: spt_quartermaster::web::update_cache::UpdateCache::new(300),
            events: events_tx,
            log_broadcast: Arc::new(spt_quartermaster::logging::LogBroadcast::new(1000)),
            reload_handles: Arc::new(spt_quartermaster::logging::init_reload_handles_only()),
            container_mgr: None,
            client_states: None,
            converging: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            fika_installed: false,
            modsync_installed: std::sync::atomic::AtomicBool::new(false),
            svm: None,
            svm_installed: std::sync::atomic::AtomicBool::new(false),
            server_transition: Arc::new(parking_lot::Mutex::new(None)),
            game_data: Arc::new(GameData::load_empty()),
            proxy_metrics: ProxyMetrics::new(),
            proxy_client,
            mod_zip_cache: spt_quartermaster::web::mod_zip_cache::ModZipCache::new(
                spt_dir.clone(),
                db_arc.clone(),
            ),
        });

        TestApp {
            db: db_arc,
            app_state,
            session_key,
            cookies: Vec::new(),
            _tmp_dir: tmp_dir,
            _mock_server: mock_server,
        }
    }
}

impl Default for TestAppBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Test app instance. Create via `TestAppBuilder`.
///
/// The tricky part: `init_service` returns an `impl Service` which can't be stored in a struct.
/// Solution: Store the components and recreate the service fresh each time via `service()`.
pub struct TestApp {
    /// Database handle for direct assertions
    pub db: Arc<Mutex<Database>>,
    app_state: web::Data<AppState>,
    session_key: Key,
    cookies: Vec<String>,
    _tmp_dir: TempDir,
    _mock_server: wiremock::MockServer,
}

impl TestApp {
    async fn make_service(
        &self,
    ) -> impl Service<actix_http::Request, Response = ServiceResponse, Error = actix_web::Error>
    {
        let app_state = self.app_state.clone();
        let session_key = self.session_key.clone();
        test::init_service(
            App::new()
                .app_data(app_state)
                .app_data(web::PayloadConfig::new(64 * 1024 * 1024))
                .wrap(middleware::NormalizePath::new(
                    middleware::TrailingSlash::MergeOnly,
                ))
                .configure(|cfg| configure_app(cfg, session_key, false, false)),
        )
        .await
    }

    /// Send a request, handling middleware errors by converting them to error responses.
    async fn send(&mut self, req: actix_http::Request) -> ServiceResponse {
        let service = self.make_service().await;
        let resp = match service.call(req).await {
            Ok(resp) => resp,
            Err(err) => {
                let response = err.error_response();
                ServiceResponse::new(
                    actix_web::test::TestRequest::default().to_http_request(),
                    response,
                )
            }
        };
        self.collect_cookies(&resp);
        resp
    }

    /// Make a GET request with current cookies. Automatically saves response cookies.
    pub async fn get(&mut self, path: &str) -> ServiceResponse {
        let mut req = TestRequest::get().uri(path);
        if !self.cookies.is_empty() {
            req = req.insert_header(("cookie", self.cookies.join("; ")));
        }
        self.send(req.to_request()).await
    }

    /// Make a POST request with form-encoded body and current cookies.
    pub async fn post_form(&mut self, path: &str, body: &str) -> ServiceResponse {
        let mut req = TestRequest::post()
            .uri(path)
            .insert_header(("content-type", "application/x-www-form-urlencoded"));
        if !self.cookies.is_empty() {
            req = req.insert_header(("cookie", self.cookies.join("; ")));
        }
        self.send(req.set_payload(body.to_string()).to_request())
            .await
    }

    /// Extract and save cookies from a response, replacing existing cookies with the same name.
    fn collect_cookies(&mut self, resp: &ServiceResponse) {
        for header in resp.headers().get_all("set-cookie") {
            if let Ok(value) = header.to_str() {
                if let Some(cookie_part) = value.split(';').next() {
                    let name = cookie_part.split('=').next().unwrap_or("");
                    self.cookies.retain(|c| !c.starts_with(&format!("{name}=")));
                    self.cookies.push(cookie_part.to_string());
                }
            }
        }
    }

    /// Extract and save cookies from a response (public, for manual use).
    pub fn save_cookies(&mut self, resp: &ServiceResponse) {
        self.collect_cookies(resp);
    }

    /// Login as a user: GET login page, extract CSRF token, POST credentials, save cookies.
    /// Panics if login fails.
    pub async fn login_as(&mut self, username: &str, password: &str) {
        // GET login page
        let resp = self.get("/quma/login").await;
        assert!(
            resp.status().is_success(),
            "login page GET failed: {}",
            resp.status()
        );

        let body = test::read_body(resp).await;
        let body_str = std::str::from_utf8(&body).expect("invalid UTF-8 in response");

        // Extract CSRF token
        let csrf_token = extract_csrf_token(body_str).expect("no CSRF token found in login page");

        // POST login
        let form_body = format!(
            "username={}&password={}&csrf_token={}",
            urlencoding::encode(username),
            urlencoding::encode(password),
            urlencoding::encode(&csrf_token)
        );

        let resp = self.post_form("/quma/login", &form_body).await;

        // Expect redirect on success (303 to /quma/)
        assert!(
            resp.status().is_redirection(),
            "login POST failed: status={}, expected redirect",
            resp.status()
        );
    }

    /// GET a page and extract the CSRF token from the HTML.
    pub async fn csrf_token_from(&mut self, path: &str) -> String {
        let resp = self.get(path).await;
        assert!(resp.status().is_success(), "GET {} failed", path);

        let body = test::read_body(resp).await;
        let body_str = std::str::from_utf8(&body).expect("invalid UTF-8");
        extract_csrf_token(body_str).unwrap_or_else(|| panic!("no CSRF token found at {}", path))
    }
}

/// Extract CSRF token from HTML. Look for `name="csrf_token"` near `value="..."`.
pub fn extract_csrf_token(html: &str) -> Option<String> {
    // Simple regex-free approach: find 'name="csrf_token"', then find 'value="..."' nearby
    let csrf_input_start = html.find(r#"name="csrf_token""#)?;
    let search_window = &html[csrf_input_start..];
    let value_start = search_window.find(r#"value=""#)? + 7; // len("value=\"") = 7
    let value_end = search_window[value_start..].find('"')?;
    Some(search_window[value_start..value_start + value_end].to_string())
}
