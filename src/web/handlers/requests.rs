use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path, Query};
use actix_web::HttpRequest;
use askama::Template;

use crate::config::FIKA_CLIENT_FORGE_ID;
use crate::db::rbac::Permission;
use crate::db::requests::{ModRequest, ModRequestView, RequestStatus, VoteComment};
use crate::forge::models::FikaCompat;
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::csrf;
use crate::web::error::WebError;
use crate::web::handlers::common::fika_compat_to_string;
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
    pending_requests: Vec<ModRequestView>,
    approved_requests: Vec<ModRequestView>,
    queued_requests: Vec<ModRequestView>,
    completed_requests: Vec<ModRequestView>,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "mods/partials/search_results.html")]
struct SearchResultsTemplate {
    results: Vec<crate::web::handlers::common::ForgeSearchResult>,
    error: Option<String>,
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

#[derive(Template)]
#[template(path = "mods/partials/request_history.html")]
struct RequestHistoryTemplate {
    entries: Vec<crate::db::requests::RequestStatusLog>,
}

#[derive(serde::Deserialize)]
pub struct TabQuery {
    pub status: String,
}

#[derive(Template)]
#[template(path = "mods/partials/request_tab_body.html")]
struct RequestTabBodyTemplate {
    user: SessionUser,
    requests: Vec<ModRequestView>,
    status: String,
    csrf_token: String,
}

// -- Helpers --

fn partition_requests(
    requests: Vec<ModRequestView>,
) -> (
    Vec<ModRequestView>,
    Vec<ModRequestView>,
    Vec<ModRequestView>,
    Vec<ModRequestView>,
) {
    let mut pending = Vec::new();
    let mut approved = Vec::new();
    let mut queued = Vec::new();
    let mut completed = Vec::new();
    for r in requests {
        match r.request.status.as_str() {
            "pending" => pending.push(r),
            "approved" => approved.push(r),
            "queued" => queued.push(r),
            _ => completed.push(r),
        }
    }
    (pending, approved, queued, completed)
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
            tracing::warn!(err = %e, "failed to fetch versions for request install");
            "Could not fetch versions for install.".to_string()
        })?;

    if versions.is_empty() {
        return Err(format!(
            "No compatible version found for SPT {}.",
            state.spt_info.spt_version
        ));
    }

    let version = versions
        .iter()
        .max_by_key(|v| v.id)
        .expect("checked non-empty above");
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
        let request_id = request.id;
        let user_id = user.user_id;
        let metadata = serde_json::json!({"request_id": request_id}).to_string();
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
                Some(&metadata),
                Some(&username),
            )?;
            db.transition_request_status(
                request_id,
                &[RequestStatus::Approved],
                RequestStatus::Queued,
                Some(user_id),
                Some("Queued for install"),
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
        let request_id = request.id;
        let tasks = state.tasks.clone();
        let forge = state.forge.clone();
        let spt_dir = state.spt_dir.clone();
        let db = state.db.clone();
        let db_edges = db.clone();
        let version = version.clone();
        let mod_name = request.mod_name.clone();
        let mod_slug = request.mod_slug.clone();
        let update_cache = state.update_cache.clone();
        let mod_zip_cache = state.mod_zip_cache.clone();
        let state_clone = state.clone().into_inner();
        let config = config.clone();

        tokio::spawn(async move {
            let result = async {
                // Install dependencies first
                let dep_db_ids = crate::ops::resolve_and_install_deps(
                    &forge,
                    &db,
                    &spt_dir,
                    &config,
                    forge_mod_id,
                    &version,
                )
                .await?;

                tasks.update_message(task_id, format!("Downloading {mod_name}…"));

                let db_id = crate::web::install::web_download_extract_and_record(
                    &forge,
                    &db,
                    &spt_dir,
                    &config,
                    forge_mod_id,
                    &mod_name,
                    mod_slug.as_deref(),
                    &version,
                )
                .await?;

                // Record dependency edges
                crate::ops::record_dep_edges(&db_edges, db_id, &dep_db_ids);

                state_clone.regenerate_modsync().await;

                Ok::<_, anyhow::Error>(())
            }
            .await;

            match result {
                Ok(()) => {
                    tracing::info!(forge_mod_id, "mod installed from request");
                    {
                        let db_req = db.clone();
                        let _ = web::block(move || {
                            let db_req = db_req.lock();
                            db_req.transition_request_status(
                                request_id,
                                &[RequestStatus::Approved],
                                RequestStatus::Installed,
                                None,
                                Some("Installed from request"),
                            )
                        })
                        .await;
                    }
                    update_cache.invalidate();
                    mod_zip_cache.invalidate();
                    state_clone.modsync_installed.store(
                        crate::config::is_modsync_installed(&spt_dir),
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    if forge_mod_id == crate::svm::SVM_FORGE_ID {
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
                    state_clone.clear_fika_items();
                    tasks.complete(task_id, "Mod installed successfully".to_string());
                }
                Err(e) => {
                    tracing::error!(forge_mod_id, err = %e, "install from request failed");
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
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let csrf_token = csrf::get_or_create_token(&session);

    let db = state.db.clone();
    let user_id = user.user_id;
    let all_requests = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(None, user_id)
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
        for rv in &all_requests {
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

    let (pending_requests, approved_requests, queued_requests, completed_requests) =
        partition_requests(all_requests);

    let tmpl = RequestsTabTemplate {
        user,
        pending_requests,
        approved_requests,
        queued_requests,
        completed_requests,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn search_mods(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<SearchQuery>,
) -> actix_web::Result<Html> {
    let _user = require_auth(&req)?;
    let q = query.q.as_deref().unwrap_or("");

    let (results, error) = crate::web::handlers::common::forge_search(&state.forge, q).await;
    let tmpl = SearchResultsTemplate { results, error };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
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
        if db.has_active_request_for_mod(forge_mod_id)? {
            return Err(
                UserFacingError("An active request for this mod already exists.".into()).into(),
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
    let all_requests = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(None, user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let (pending_requests, approved_requests, queued_requests, completed_requests) =
        partition_requests(all_requests);

    let tmpl = RequestsTabTemplate {
        user,
        pending_requests,
        approved_requests,
        queued_requests,
        completed_requests,
        csrf_token,
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

    let status: RequestStatus = request
        .status
        .parse()
        .map_err(|e: String| WebError::Internal(anyhow::anyhow!(e)))?;
    if status != RequestStatus::Pending {
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

    let comment = form.comment.as_deref().filter(|s| !s.trim().is_empty());

    // Determine allowed source statuses and target
    let (expected_from, new_status) = if action == "approve" {
        (vec![RequestStatus::Pending], RequestStatus::Approved)
    } else {
        // reject allowed from pending or approved
        (
            vec![RequestStatus::Pending, RequestStatus::Approved],
            RequestStatus::Rejected,
        )
    };

    let resolved_by = user.user_id;
    let comment_owned = comment.map(|s| s.to_string());

    // Atomic: set resolver + transition in one web::block (one lock acquisition)
    let db = state.db.clone();
    let transitioned = web::block({
        let comment_owned = comment_owned.clone();
        move || {
            let db = db.lock();
            db.set_request_resolver(request_id, resolved_by, comment_owned.as_deref())?;
            db.transition_request_status(
                request_id,
                &expected_from,
                new_status,
                Some(resolved_by),
                comment_owned.as_deref(),
            )
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if !transitioned {
        return Err(WebError::BadRequest(
            "This request has already been resolved or its status changed.".to_string(),
        )
        .into());
    }

    let mut message = if action == "approve" {
        "Request approved.".to_string()
    } else {
        "Request rejected.".to_string()
    };

    if action == "approve" && form.install.as_deref() == Some("true") {
        let request = web::block({
            let db = state.db.clone();
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

    let current: RequestStatus = request
        .status
        .parse()
        .map_err(|e: String| WebError::Internal(anyhow::anyhow!(e)))?;
    if current != RequestStatus::Approved {
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
            db.list_approved_request_ids()
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
    let all_requests = web::block(move || {
        let db = db.lock();
        db.list_mod_requests(None, user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let (pending_requests, approved_requests, queued_requests, completed_requests) =
        partition_requests(all_requests);

    let tmpl = RequestsTabTemplate {
        user,
        pending_requests,
        approved_requests,
        queued_requests,
        completed_requests,
        csrf_token,
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

pub async fn reopen_request(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<csrf::CsrfForm>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::RequestsResolve)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let request_id = path.into_inner();
    let user_id = user.user_id;

    let transitioned = web::block({
        let db = state.db.clone();
        move || {
            let db = db.lock();
            db.transition_request_status(
                request_id,
                &[RequestStatus::Rejected],
                RequestStatus::Pending,
                Some(user_id),
                Some("Reopened"),
            )
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if !transitioned {
        return Err(
            WebError::BadRequest("Only rejected requests can be reopened.".to_string()).into(),
        );
    }

    let views = web::block({
        let db = state.db.clone();
        move || {
            let db = db.lock();
            db.list_mod_requests(None, user_id)
        }
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
        message: Some("Request reopened.".to_string()),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn request_history(
    state: Data<AppState>,
    req: HttpRequest,
    path: Path<i64>,
) -> actix_web::Result<Html> {
    let _user = require_auth(&req)?;
    let request_id = path.into_inner();

    let db = state.db.clone();
    let entries = web::block(move || {
        let db = db.lock();
        db.get_request_status_log(request_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = RequestHistoryTemplate { entries };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn request_tab_body(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    query: Query<TabQuery>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let csrf_token = csrf::get_or_create_token(&session);
    let user_id = user.user_id;
    let status_filter = query.status.clone();

    let db_status = match status_filter.as_str() {
        "completed" => None, // completed = installed + rejected, partition in Rust
        other => Some(other.to_string()),
    };
    let db = state.db.clone();
    let requests = web::block(move || {
        let db = db.lock();
        if let Some(ref s) = db_status {
            db.list_mod_requests(Some(s), user_id)
        } else {
            // "completed" = all terminal states; fetch all, keep non-active
            let all = db.list_mod_requests(None, user_id)?;
            Ok(all
                .into_iter()
                .filter(|r| {
                    let s = r.request.status.as_str();
                    s != "pending" && s != "approved" && s != "queued"
                })
                .collect())
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = RequestTabBodyTemplate {
        user,
        requests,
        status: status_filter,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

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
