use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path, Query};
use actix_web::HttpRequest;
use askama::Template;

use crate::config::FIKA_CLIENT_FORGE_ID;
use crate::db::rbac::Permission;
use crate::db::requests::{ModRequest, ModRequestView, VoteComment};
use crate::forge::models::FikaCompat;
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::csrf;
use crate::web::error::WebError;
use crate::web::state::AppState;

/// A user-facing error whose message is safe to render in HTML responses.
///
/// Wrapping intentional validation messages in this type lets the error-mapping
/// code distinguish them from unexpected DB/IO errors that must NOT be shown
/// to the user.
#[derive(Debug)]
struct UserFacingError(String);

impl std::fmt::Display for UserFacingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UserFacingError {}

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

// -- Query / Form structs --

#[derive(serde::Deserialize)]
pub struct StatusQuery {
    pub status: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct CreateRequestForm {
    pub forge_mod_id: i64,
    pub reason: Option<String>,
    pub csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct VoteForm {
    pub upvote: String,
    pub comment: Option<String>,
    pub csrf_token: String,
}

#[derive(serde::Deserialize)]
pub struct ResolveForm {
    pub action: String,
    pub comment: Option<String>,
    pub install: Option<String>,
    pub csrf_token: String,
}

// -- Templates --

#[derive(Template)]
#[template(path = "mods/partials/requests.html")]
struct RequestsTabTemplate {
    user: SessionUser,
    requests: Vec<ModRequestView>,
    active_filter: String,
    csrf_token: String,
    has_uninstalled_approved: bool,
}

#[derive(Template)]
#[template(path = "mods/partials/search_results.html")]
struct SearchResultsTemplate {
    results: Vec<SearchResult>,
    error: Option<String>,
}

pub struct SearchResult {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub fika_compatible: String,
}

#[derive(Template)]
#[template(path = "mods/partials/request_card.html")]
struct RequestCardTemplate {
    user: SessionUser,
    r: ModRequestView,
    csrf_token: String,
    message: Option<String>,
}

#[derive(Template)]
#[template(path = "mods/partials/vote_comments.html")]
struct VoteCommentsTemplate {
    comments: Vec<VoteComment>,
}

// -- Helpers --

pub fn parse_forge_url(input: &str) -> Option<i64> {
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

fn strip_html_tags(html: &str) -> String {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"<[^>]+>").expect("valid regex"));
    RE.replace_all(html, "").trim().to_string()
}

fn is_cache_stale(forge_cached_at: &str, ttl_secs: u64) -> bool {
    use chrono::{NaiveDateTime, Utc};
    let cached = NaiveDateTime::parse_from_str(forge_cached_at, "%Y-%m-%d %H:%M:%S")
        .map(|dt| dt.and_utc())
        .or_else(|_| {
            chrono::DateTime::parse_from_rfc3339(forge_cached_at).map(|dt| dt.with_timezone(&Utc))
        });
    match cached {
        Ok(dt) => {
            let age = Utc::now().signed_duration_since(dt);
            age.num_seconds() > ttl_secs as i64
        }
        Err(_) => {
            tracing::warn!(
                forge_cached_at,
                "failed to parse cache timestamp, treating as stale"
            );
            true
        }
    }
}

// TODO(debt): this duplicates install logic from mods::install_mod — extract a shared helper
async fn trigger_install_for_request(
    state: &Data<AppState>,
    request: &ModRequest,
    user: &SessionUser,
) -> Result<String, String> {
    let forge_mod_id = request.forge_mod_id;

    let versions = state
        .forge
        .get_versions(forge_mod_id, Some(&state.spt_info.spt_version))
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "failed to fetch versions for request install");
            "Could not fetch versions for install.".to_string()
        })?;

    if versions.is_empty() {
        return Err(format!(
            "No compatible version found for SPT {}.",
            state.spt_info.spt_version
        ));
    }

    let version = versions.last().expect("checked non-empty above");
    let mut message = String::new();

    {
        let db = state.db.clone();
        let fika_installed = web::block(move || {
            let db = db.lock();
            Ok::<_, anyhow::Error>(db.get_mod_by_forge_id(FIKA_CLIENT_FORGE_ID)?.is_some())
        })
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or(false);

        if fika_installed {
            match &version.fika_compatibility {
                Some(FikaCompat::Incompatible) => {
                    message = format!(
                        "Warning: {} v{} is marked as Fika INCOMPATIBLE. ",
                        request.mod_name, version.version
                    );
                }
                Some(FikaCompat::Unknown) => {
                    message = format!(
                        "Note: Fika compatibility for {} v{} is unknown. ",
                        request.mod_name, version.version
                    );
                }
                _ => {}
            }
        }
    }

    let config = state.config_cloned();
    let should_queue = crate::queue::should_queue(
        &config,
        false,
        &state.spt_dir,
        state.container_mgr.as_deref(),
    )
    .await
    .unwrap_or(false);

    if should_queue {
        let db = state.db.clone();
        let mod_name = request.mod_name.clone();
        let version_id = version.id;
        let username = user.username.clone();
        let already_queued = web::block(move || {
            let db = db.lock();
            if db.has_pending_op(forge_mod_id, crate::db::users::QueueAction::Install)? {
                return Ok::<bool, rusqlite::Error>(true);
            }
            db.insert_pending_op(
                crate::db::users::QueueAction::Install,
                forge_mod_id,
                Some(version_id),
                &mod_name,
                None,
                Some(&username),
            )?;
            Ok(false)
        })
        .await
        .map_err(|e| format!("Failed to queue install: {e}"))
        .and_then(|r| r.map_err(|e| format!("Failed to queue install: {e}")))?;
        if already_queued {
            message += "Already queued for install.";
        } else {
            message += "Queued for install.";
        }
    } else {
        let Some(task_id) =
            state
                .tasks
                .start_if_not_running("Installing", &request.mod_name, forge_mod_id)
        else {
            message += "Install already in progress.";
            return Ok(message);
        };
        let tasks = state.tasks.clone();
        let forge = state.forge.clone();
        let spt_dir = state.spt_dir.clone();
        let db = state.db.clone();
        let version = version.clone();
        let mod_name = request.mod_name.clone();
        let mod_slug = request.mod_slug.clone();
        let update_cache = state.update_cache.clone();
        let state_clone = state.clone().into_inner();
        let config = config.clone();

        tokio::spawn(async move {
            let result = async {
                let link = version
                    .link
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("version has no download link"))?;
                let tmp_dir = tempfile::tempdir()?;
                let archive_path = tmp_dir.path().join("mod.zip");
                forge.download_file(link, &archive_path).await?;

                let spt_dir2 = spt_dir.clone();
                let extracted = actix_web::web::block(move || {
                    crate::spt::mods::extract_mod(&archive_path, &spt_dir2)
                })
                .await??;

                let version_id = version.id;
                let version_str = version.version.clone();
                let spt_dir2 = spt_dir.clone();
                let spt_dir3 = spt_dir.clone();
                let db2 = db.clone();
                let db3 = db.clone();
                let config2 = config.clone();
                let db_id = actix_web::web::block(move || {
                    let db = db.lock();
                    let tx = db.begin_transaction()?;
                    let db_id = db.insert_mod(
                        forge_mod_id,
                        version_id,
                        &mod_name,
                        mod_slug.as_deref(),
                        &version_str,
                    )?;
                    for file in &extracted {
                        db.insert_file(
                            db_id,
                            &file.path,
                            Some(&file.hash),
                            Some(file.size as i64),
                        )?;
                    }
                    tx.commit()?;
                    Ok::<_, anyhow::Error>(db_id)
                })
                .await??;

                let _ = actix_web::web::block(move || {
                    crate::ops::scan_and_record_runtime_files(&db2, db_id, &spt_dir2)
                })
                .await;

                let _ = actix_web::web::block(move || {
                    let db = db3.lock();
                    crate::modsync::regenerate_if_enabled(&spt_dir3, &config2, &db)
                })
                .await;

                Ok::<_, anyhow::Error>(())
            }
            .await;

            match result {
                Ok(()) => {
                    tracing::info!(forge_mod_id, "mod installed from request");
                    update_cache.invalidate();
                    state_clone.modsync_installed.store(
                        crate::config::is_modsync_installed(&spt_dir),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    if forge_mod_id == 236 {
                        state_clone
                            .svm_installed
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        if let Some(ref svm_lock) = state_clone.svm {
                            if let Some(mgr) = crate::svm::SvmManager::detect(&spt_dir) {
                                *svm_lock.write() = mgr;
                            }
                        }
                        tracing::info!("SVM installed via request — config editor reinitialized");
                    }
                    tasks.complete(task_id, "Mod installed successfully".to_string());
                }
                Err(e) => {
                    tracing::error!(forge_mod_id, error = %e, "install from request failed");
                    tasks.fail(task_id, format!("Install failed: {e}"));
                }
            }
        });
        message += "Installing now.";
    }
    Ok(message)
}

// -- Handlers --

pub async fn requests_tab(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<StatusQuery>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let csrf_token = csrf::get_or_create_token(&session);

    let filter = query
        .status
        .clone()
        .unwrap_or_else(|| "pending".to_string());
    let filter_param = if filter == "all" {
        None
    } else {
        Some(filter.as_str())
    };

    let db = state.db.clone();
    let user_id = user.user_id;
    let filter_owned = filter_param.map(|s| s.to_string());
    let requests = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(filter_owned.as_deref(), user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    // Spawn background cache refresh for stale entries (capped to avoid
    // unbounded concurrent Forge API requests; remaining entries refresh
    // on subsequent page loads).
    const MAX_BACKGROUND_REFRESHES: usize = 3;
    if let Some(ttl) = state.config().forge_cache_ttl.filter(|&t| t > 0) {
        let mut refresh_count = 0usize;
        for rv in &requests {
            if refresh_count >= MAX_BACKGROUND_REFRESHES {
                break;
            }
            if is_cache_stale(&rv.request.forge_cached_at, ttl) {
                refresh_count += 1;
                let db = state.db.clone();
                let forge = state.forge.clone();
                let request_id = rv.request.id;
                let forge_mod_id = rv.request.forge_mod_id;
                tokio::spawn(async move {
                    match forge.get_mod(forge_mod_id, false).await {
                        Ok(m) => {
                            let fc = fika_compat_to_string(&m.fika_compatibility);
                            let clean_desc = m.description.as_deref().map(strip_html_tags);
                            let _ = web::block(move || {
                                let db = db.lock();
                                db.update_mod_request_cache(
                                    request_id,
                                    &m.name,
                                    m.slug.as_deref(),
                                    clean_desc.as_deref(),
                                    &fc,
                                )
                            })
                            .await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                forge_mod_id,
                                err = %e,
                                "failed to refresh Forge cache for mod request"
                            );
                        }
                    }
                });
            }
        }
    }

    let has_uninstalled_approved = requests
        .iter()
        .any(|r| r.request.status == "approved" && !r.is_installed);
    let tmpl = RequestsTabTemplate {
        user,
        requests,
        active_filter: filter,
        csrf_token,
        has_uninstalled_approved,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn search_mods(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<SearchQuery>,
) -> actix_web::Result<Html> {
    let _user = require_auth(&req)?;
    let q = query.q.as_deref().unwrap_or("").trim().to_string();

    // Check for direct ID or URL first — single-digit mod IDs are valid
    if let Some(mod_id) = parse_forge_url(&q) {
        match state.forge.get_mod(mod_id, false).await {
            Ok(m) => {
                let tmpl = SearchResultsTemplate {
                    results: vec![SearchResult {
                        id: m.id,
                        name: m.name,
                        description: m.description,
                        fika_compatible: fika_compat_to_string(&m.fika_compatibility),
                    }],
                    error: None,
                };
                return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
            }
            Err(_) => {
                let tmpl = SearchResultsTemplate {
                    results: vec![],
                    error: Some(format!("Mod with ID {mod_id} not found on Forge.")),
                };
                return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
            }
        }
    }

    // Only apply min-length guard for text searches (not numeric IDs/URLs handled above)
    if q.len() < 2 {
        let tmpl = SearchResultsTemplate {
            results: vec![],
            error: None,
        };
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    match state.forge.search_mods(&q).await {
        Ok(mods) => {
            let results = mods
                .into_iter()
                .map(|m| SearchResult {
                    id: m.id,
                    name: m.name,
                    description: m.description,
                    fika_compatible: fika_compat_to_string(&m.fika_compatibility),
                })
                .collect();
            let tmpl = SearchResultsTemplate {
                results,
                error: None,
            };
            Ok(Html::new(tmpl.render().map_err(WebError::from)?))
        }
        Err(_) => {
            let tmpl = SearchResultsTemplate {
                results: vec![],
                error: Some("Could not reach SPT Forge. Try again later.".to_string()),
            };
            Ok(Html::new(tmpl.render().map_err(WebError::from)?))
        }
    }
}

pub async fn create_request(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<CreateRequestForm>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let forge_mod_id = form.forge_mod_id;
    let csrf_token = csrf::get_or_create_token(&session);
    let user_id = user.user_id;

    // Fetch fresh mod info from Forge (before taking the DB lock)
    let mod_info = state
        .forge
        .get_mod(forge_mod_id, false)
        .await
        .map_err(|_| WebError::BadRequest("Could not verify mod on SPT Forge.".to_string()))?;

    let fc = fika_compat_to_string(&mod_info.fika_compatibility);
    let reason = form
        .reason
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());
    let mod_name = mod_info.name.clone();
    let mod_slug = mod_info.slug.clone();
    let mod_desc = mod_info.description.as_deref().map(strip_html_tags);

    // Atomically check installed + pending + insert under a single DB lock
    // to prevent TOCTOU races on duplicate request creation.
    let db = state.db.clone();
    web::block(move || {
        let db = db.lock();

        if db.get_mod_by_forge_id(forge_mod_id)?.is_some() {
            return Err(UserFacingError("This mod is already installed.".into()).into());
        }
        if db.has_pending_request_for_mod(forge_mod_id)? {
            return Err(
                UserFacingError("A pending request for this mod already exists.".into()).into(),
            );
        }

        db.create_mod_request(
            user_id,
            forge_mod_id,
            &mod_name,
            mod_slug.as_deref(),
            mod_desc.as_deref(),
            &fc,
            reason.as_deref(),
        )?;
        Ok(())
    })
    .await
    .map_err(WebError::from)?
    .map_err(|e: anyhow::Error| {
        if e.downcast_ref::<UserFacingError>().is_some() {
            WebError::BadRequest(e.to_string())
        } else {
            WebError::Internal(e)
        }
    })?;

    // Return the updated requests tab
    let db = state.db.clone();
    let requests = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(Some("pending"), user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = RequestsTabTemplate {
        user,
        requests,
        active_filter: "pending".to_string(),
        csrf_token,
        has_uninstalled_approved: false,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn vote(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<VoteForm>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let request_id = path.into_inner();
    let upvote = form.upvote == "true";
    let comment = form.comment.as_deref().filter(|s| !s.trim().is_empty());

    // Check request exists and is pending
    let db = state.db.clone();
    let request = web::block({
        let db = db.clone();
        move || {
            let db = db.lock();
            db.get_mod_request(request_id)
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    if request.status != "pending" {
        return Err(WebError::BadRequest(
            "Voting is only allowed on pending requests.".to_string(),
        )
        .into());
    }

    // Check if user already voted the same way (toggle off)
    let db = state.db.clone();
    let user_id = user.user_id;
    let existing_vote = web::block({
        let db = db.clone();
        move || {
            let db = db.lock();
            db.get_vote(request_id, user_id)
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let db = state.db.clone();
    if let Some(existing) = existing_vote {
        if existing.upvote == upvote {
            // Toggle off — remove vote
            web::block(move || {
                let db = db.lock();
                db.delete_vote(request_id, user_id)
            })
            .await
            .map_err(WebError::from)?
            .map_err(WebError::from)?;
        } else {
            // Change vote direction
            let comment_owned = comment.map(|s| s.to_string());
            web::block(move || {
                let db = db.lock();
                db.upsert_vote(request_id, user_id, upvote, comment_owned.as_deref())
            })
            .await
            .map_err(WebError::from)?
            .map_err(WebError::from)?;
        }
    } else {
        // New vote
        let comment_owned = comment.map(|s| s.to_string());
        web::block(move || {
            let db = db.lock();
            db.upsert_vote(request_id, user_id, upvote, comment_owned.as_deref())
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;
    }

    // Re-fetch the request view to render the updated card
    let db = state.db.clone();
    let views = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(None, user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let rv = views
        .into_iter()
        .find(|v| v.request.id == request_id)
        .ok_or(WebError::NotFound)?;

    let csrf_token = csrf::get_or_create_token(&session);
    let tmpl = RequestCardTemplate {
        user,
        r: rv,
        csrf_token,
        message: None,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn vote_comments(
    state: Data<AppState>,
    req: HttpRequest,
    path: Path<i64>,
) -> actix_web::Result<Html> {
    let _user = require_auth(&req)?;
    let request_id = path.into_inner();

    let db = state.db.clone();
    let comments = web::block(move || {
        let db = db.lock();
        db.list_vote_comments(request_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = VoteCommentsTemplate { comments };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn resolve_request(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<ResolveForm>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::RequestsResolve)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let request_id = path.into_inner();
    let action = form.action.as_str();
    if action != "approve" && action != "reject" {
        return Err(WebError::BadRequest("Invalid action.".to_string()).into());
    }

    let status = if action == "approve" {
        "approved"
    } else {
        "rejected"
    };
    let comment = form.comment.as_deref().filter(|s| !s.trim().is_empty());

    // Resolve the request (only if pending)
    let db = state.db.clone();
    let resolved_by = user.user_id;
    let comment_owned = comment.map(|s| s.to_string());
    let status_owned = status.to_string();
    let rows = web::block({
        let db = db.clone();
        let status_owned = status_owned.clone();
        let comment_owned = comment_owned.clone();
        move || {
            let db = db.lock();
            db.resolve_mod_request(
                request_id,
                &status_owned,
                resolved_by,
                comment_owned.as_deref(),
            )
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if rows == 0 {
        return Err(
            WebError::BadRequest("This request has already been resolved.".to_string()).into(),
        );
    }

    let mut message = if action == "approve" {
        "Request approved.".to_string()
    } else {
        "Request rejected.".to_string()
    };

    if action == "approve" && form.install.as_deref() == Some("true") {
        let request = web::block({
            let db = db.clone();
            move || {
                let db = db.lock();
                db.get_mod_request(request_id)
            }
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?
        .ok_or(WebError::NotFound)?;

        match trigger_install_for_request(&state, &request, &user).await {
            Ok(msg) => message = format!("Approved. {msg}"),
            Err(msg) => message = format!("Approved. {msg}"),
        }
    }

    // Re-fetch the request view
    let db = state.db.clone();
    let user_id = user.user_id;
    let views = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(None, user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let rv = views
        .into_iter()
        .find(|v| v.request.id == request_id)
        .ok_or(WebError::NotFound)?;

    let csrf_token = csrf::get_or_create_token(&session);
    let tmpl = RequestCardTemplate {
        user,
        r: rv,
        csrf_token,
        message: Some(message),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn install_from_request(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<csrf::CsrfForm>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsInstall)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let request_id = path.into_inner();
    let db = state.db.clone();
    let request = web::block({
        let db = db.clone();
        move || {
            let db = db.lock();
            db.get_mod_request(request_id)
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    if request.status != "approved" {
        return Err(
            WebError::BadRequest("Only approved requests can be installed.".to_string()).into(),
        );
    }

    let already_installed = web::block({
        let db = db.clone();
        let forge_mod_id = request.forge_mod_id;
        move || {
            let db = db.lock();
            Ok::<_, anyhow::Error>(db.get_mod_by_forge_id(forge_mod_id)?.is_some())
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if already_installed {
        return Err(WebError::BadRequest("This mod is already installed.".to_string()).into());
    }

    let message = match trigger_install_for_request(&state, &request, &user).await {
        Ok(msg) | Err(msg) => msg,
    };

    let user_id = user.user_id;
    let views = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(None, user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let rv = views
        .into_iter()
        .find(|v| v.request.id == request_id)
        .ok_or(WebError::NotFound)?;

    let csrf_token = csrf::get_or_create_token(&session);
    let tmpl = RequestCardTemplate {
        user,
        r: rv,
        csrf_token,
        message: Some(message),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn install_all_approved(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<csrf::CsrfForm>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ModsInstall)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let db = state.db.clone();
    let request_ids = web::block({
        let db = db.clone();
        move || {
            let db = db.lock();
            db.list_approved_uninstalled_request_ids()
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let mut installed = 0usize;
    let mut failed = 0usize;
    for rid in &request_ids {
        let request = web::block({
            let db = db.clone();
            let rid = *rid;
            move || {
                let db = db.lock();
                db.get_mod_request(rid)
            }
        })
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

        if let Some(request) = request {
            if trigger_install_for_request(&state, &request, &user)
                .await
                .is_ok()
            {
                installed += 1;
            } else {
                failed += 1;
            }
        }
    }

    let message = if request_ids.is_empty() {
        "No approved mods to install.".to_string()
    } else if failed == 0 {
        format!("{installed} mod(s) queued/installing.")
    } else {
        format!("{installed} mod(s) queued/installing, {failed} failed.")
    };

    let csrf_token = csrf::get_or_create_token(&session);
    let user_id = user.user_id;
    let requests = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(Some("approved"), user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let has_uninstalled_approved = requests.iter().any(|r| !r.is_installed);
    let tmpl = RequestsTabTemplate {
        user,
        requests,
        active_filter: "approved".to_string(),
        csrf_token,
        has_uninstalled_approved,
    };

    let toast_class = if failed > 0 {
        "toast-warning"
    } else {
        "toast-success"
    };
    let mut html = tmpl.render().map_err(WebError::from)?;
    html = format!(
        "<div class=\"toast {toast_class}\" style=\"margin-bottom:0.5rem\">{message}</div>{html}"
    );
    Ok(Html::new(html))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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

    #[test]
    fn cache_stale_old_datetime() {
        assert!(is_cache_stale("2020-01-01 00:00:00", 86400));
    }

    #[test]
    fn cache_stale_recent_datetime() {
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        assert!(!is_cache_stale(&now, 86400));
    }

    #[test]
    fn cache_stale_rfc3339_format() {
        assert!(is_cache_stale("2020-01-01T00:00:00+00:00", 86400));
    }

    #[test]
    fn strip_html_tags_removes_tags() {
        assert_eq!(strip_html_tags("<p>Hello <b>world</b></p>"), "Hello world");
    }

    #[test]
    fn strip_html_tags_handles_nested_html() {
        assert_eq!(
            strip_html_tags("<div><p>Fika is a <a href=\"#\">cooperative</a> mod</p></div>"),
            "Fika is a cooperative mod"
        );
    }

    #[test]
    fn strip_html_tags_preserves_plain_text() {
        assert_eq!(strip_html_tags("no html here"), "no html here");
    }

    #[test]
    fn strip_html_tags_handles_empty() {
        assert_eq!(strip_html_tags(""), "");
    }
}
