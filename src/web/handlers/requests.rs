use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path, Query};
use actix_web::HttpRequest;
use askama::Template;

use crate::db::requests::{ModRequestView, VoteComment};
use crate::forge::models::FikaCompat;
use crate::web::auth::{require_auth, require_capability, SessionUser};
use crate::web::csrf;
use crate::web::error::WebError;
use crate::web::state::AppState;

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
#[allow(dead_code)]
struct RequestsTabTemplate {
    user: SessionUser,
    requests: Vec<ModRequestView>,
    active_filter: String,
    csrf_token: String,
}

#[derive(Template)]
#[template(path = "mods/partials/search_results.html")]
#[allow(dead_code)]
struct SearchResultsTemplate {
    results: Vec<SearchResult>,
    error: Option<String>,
}

#[allow(dead_code)]
pub struct SearchResult {
    pub id: i64,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub fika_compatible: String,
}

#[derive(Template)]
#[template(path = "mods/partials/request_card.html")]
#[allow(dead_code)]
struct RequestCardTemplate {
    user: SessionUser,
    r: ModRequestView,
    csrf_token: String,
    message: Option<String>,
}

#[derive(Template)]
#[template(path = "mods/partials/vote_comments.html")]
#[allow(dead_code)]
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

fn fika_compat_to_string(fc: &Option<FikaCompat>) -> String {
    match fc {
        Some(FikaCompat::Compatible) => "compatible".to_string(),
        Some(FikaCompat::Incompatible) => "incompatible".to_string(),
        _ => "unknown".to_string(),
    }
}

fn is_cache_stale(forge_cached_at: &str, ttl_secs: u64) -> bool {
    use chrono::{NaiveDateTime, Utc};
    let cached = NaiveDateTime::parse_from_str(forge_cached_at, "%Y-%m-%d %H:%M:%S")
        .map(|dt| dt.and_utc())
        .unwrap_or_else(|_| {
            chrono::DateTime::parse_from_rfc3339(forge_cached_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now())
        });
    let age = Utc::now().signed_duration_since(cached);
    age.num_seconds() > ttl_secs as i64
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

    // Spawn background cache refresh for stale entries
    if let Some(ttl) = state.config.forge_cache_ttl {
        for rv in &requests {
            if is_cache_stale(&rv.request.forge_cached_at, ttl) {
                let db = state.db.clone();
                let forge = state.forge.clone();
                let request_id = rv.request.id;
                let forge_mod_id = rv.request.forge_mod_id;
                tokio::spawn(async move {
                    match forge.get_mod(forge_mod_id, false).await {
                        Ok(m) => {
                            let fc = fika_compat_to_string(&m.fika_compatibility);
                            let _ = web::block(move || {
                                let db = db.lock();
                                db.update_mod_request_cache(
                                    request_id,
                                    &m.name,
                                    m.slug.as_deref(),
                                    m.description.as_deref(),
                                    &fc,
                                )
                            })
                            .await;
                        }
                        Err(e) => {
                            tracing::warn!(
                                forge_mod_id,
                                error = %e,
                                "failed to refresh Forge cache for mod request"
                            );
                        }
                    }
                });
            }
        }
    }

    let tmpl = RequestsTabTemplate {
        user,
        requests,
        active_filter: filter,
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
    let q = query.q.as_deref().unwrap_or("").trim().to_string();

    if q.len() < 2 {
        let tmpl = SearchResultsTemplate {
            results: vec![],
            error: None,
        };
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    // Check for direct ID or URL
    if let Some(mod_id) = parse_forge_url(&q) {
        match state.forge.get_mod(mod_id, false).await {
            Ok(m) => {
                let tmpl = SearchResultsTemplate {
                    results: vec![SearchResult {
                        id: m.id,
                        name: m.name,
                        slug: m.slug,
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

    match state.forge.search_mods(&q).await {
        Ok(mods) => {
            let results = mods
                .into_iter()
                .map(|m| SearchResult {
                    id: m.id,
                    name: m.name,
                    slug: m.slug,
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

    // Check if mod is already installed
    let db = state.db.clone();
    let is_installed = web::block(move || {
        let db = db.lock();
        Ok::<_, anyhow::Error>(db.get_mod_by_forge_id(forge_mod_id)?.is_some())
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if is_installed {
        return Err(WebError::BadRequest("This mod is already installed.".to_string()).into());
    }

    // Check for existing pending request
    let db = state.db.clone();
    let has_pending = web::block(move || {
        let db = db.lock();
        db.has_pending_request_for_mod(forge_mod_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if has_pending {
        return Err(WebError::BadRequest(
            "A pending request for this mod already exists.".to_string(),
        )
        .into());
    }

    // Fetch fresh mod info from Forge
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
    let mod_desc = mod_info.description.clone();

    let db = state.db.clone();
    web::block(move || {
        let db = db.lock();
        db.create_mod_request(
            user_id,
            forge_mod_id,
            &mod_name,
            mod_slug.as_deref(),
            mod_desc.as_deref(),
            &fc,
            reason.as_deref(),
        )
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

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
    require_capability(&user, crate::db::users::Role::can_manage_mods)?;
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

    // Install-on-approve
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

        let forge_mod_id = request.forge_mod_id;

        match state
            .forge
            .get_versions(forge_mod_id, Some(&state.spt_info.spt_version))
            .await
        {
            Ok(versions) if !versions.is_empty() => {
                let version = versions.last().unwrap();
                let should_queue = crate::queue::should_queue(
                    &state.config,
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
                    web::block(move || {
                        let db = db.lock();
                        db.insert_pending_op(
                            "install",
                            forge_mod_id,
                            Some(version_id),
                            &mod_name,
                            None,
                            Some(&username),
                        )
                    })
                    .await
                    .map_err(WebError::from)?
                    .map_err(WebError::from)?;
                    message = "Approved and queued for install.".to_string();
                } else {
                    // Direct install via async task (same pattern as mods::install_mod)
                    let task_id = state
                        .tasks
                        .start("Installing", &request.mod_name, forge_mod_id);
                    let tasks = state.tasks.clone();
                    let forge = state.forge.clone();
                    let spt_dir = state.spt_dir.clone();
                    let db = state.db.clone();
                    let version = version.clone();
                    let mod_name = request.mod_name.clone();
                    let mod_slug = request.mod_slug.clone();
                    let update_cache = state.update_cache.clone();

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
                            let db2 = db.clone();
                            let db_id = actix_web::web::block(move || {
                                let db = db.lock();
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
                                Ok::<_, anyhow::Error>(db_id)
                            })
                            .await??;

                            let _ = actix_web::web::block(move || {
                                crate::ops::scan_and_record_runtime_files(&db2, db_id, &spt_dir2)
                            })
                            .await;

                            Ok::<_, anyhow::Error>(())
                        }
                        .await;

                        match result {
                            Ok(()) => {
                                tracing::info!(forge_mod_id, "mod installed via request approval");
                                update_cache.invalidate();
                                tasks.complete(task_id, "Mod installed successfully".to_string());
                            }
                            Err(e) => {
                                tracing::error!(forge_mod_id, error = %e, "install from request approval failed");
                                tasks.fail(task_id, format!("Install failed: {e}"));
                            }
                        }
                    });
                    message = "Approved and installing now.".to_string();
                }
            }
            Ok(_) => {
                message = format!(
                    "Approved but no compatible version found for SPT {}.",
                    state.spt_info.spt_version
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to fetch versions for install-on-approve");
                message = "Approved. Could not fetch versions for auto-install.".to_string();
            }
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
