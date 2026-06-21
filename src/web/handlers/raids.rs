use actix_session::Session;
use actix_web::web::{self, Data, Html, Path, Query};
use actix_web::HttpRequest;
use askama::Template;
use serde::Deserialize;

use crate::db::raids::{Raid, RaidKill, ServerRaidStats, UserRaidStats};
use crate::web::auth::require_auth;
use crate::web::csrf;
use crate::web::error::WebError;
use crate::web::flash::{take_flash, FlashMessage};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

#[derive(Template)]
#[template(path = "raids/server.html")]
struct ServerRaidsPageTemplate {
    user: crate::web::auth::SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    stats: ServerRaidStats,
    active_raids: Vec<(Raid, String)>,
}

#[derive(Template)]
#[template(path = "raids/player.html")]
struct PlayerRaidsPageTemplate {
    user: crate::web::auth::SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    profile_username: String,
    stats: UserRaidStats,
    raids: Vec<Raid>,
    offset: i64,
    has_more: bool,
}

#[derive(Template)]
#[template(path = "raids/detail.html")]
struct RaidDetailPageTemplate {
    user: crate::web::auth::SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    raid: Raid,
    kills: Vec<RaidKill>,
    squad: Vec<(Raid, String)>,
    raid_username: String,
    snapshot_sizes: Vec<(String, i64)>,
}

#[derive(Template)]
#[template(path = "raids/partials/active.html")]
struct ActiveRaidsPartialTemplate {
    active_raids: Vec<(Raid, String)>,
}

#[derive(Template)]
#[template(path = "raids/partials/recent.html")]
struct RecentRaidsPartialTemplate {
    recent_raids: Vec<(Raid, String)>,
}

#[derive(Deserialize)]
pub struct RaidsQuery {
    #[serde(default)]
    offset: Option<i64>,
}

pub async fn server_raids_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = csrf::get_or_create_token(&session);

    let db = state.db.clone();

    let (stats, active_raids) = web::block(move || {
        let db = db.lock();
        let stats = db.get_server_raid_stats()?;
        let active = db.get_active_raids()?;
        Ok::<_, anyhow::Error>((stats, active))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = ServerRaidsPageTemplate {
        user,
        flash,
        csrf_token,
        nav: NavContext::from_state(&state),
        stats,
        active_raids,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn player_raids_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<String>,
    query: Query<RaidsQuery>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = csrf::get_or_create_token(&session);
    let profile_username = path.into_inner();
    let offset = query.offset.unwrap_or(0);

    let db = state.db.clone();
    let lookup_username = profile_username.clone();

    let (_target_user_id, stats, raids) = web::block(move || {
        let db = db.lock();
        let target_user = db
            .get_user_by_username(&lookup_username)?
            .ok_or_else(|| anyhow::anyhow!("user not found"))?;
        let stats = db.get_user_raid_stats(target_user.id)?;
        let raids = db.get_raids_for_user(target_user.id, 26, offset)?;
        Ok::<_, anyhow::Error>((target_user.id, stats, raids))
    })
    .await
    .map_err(WebError::from)?
    .map_err(|_| WebError::NotFound)?;

    let has_more = raids.len() > 25;
    let raids = raids.into_iter().take(25).collect();

    let tmpl = PlayerRaidsPageTemplate {
        user,
        flash,
        csrf_token,
        nav: NavContext::from_state(&state),
        profile_username,
        stats,
        raids,
        offset,
        has_more,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn raid_detail_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = csrf::get_or_create_token(&session);
    let raid_id = path.into_inner();

    let db = state.db.clone();

    let (raid, kills, squad, raid_username, snapshot_sizes) = web::block(move || {
        let db = db.lock();
        let (raid, kills) = db
            .get_raid_with_kills(raid_id)?
            .ok_or_else(|| anyhow::anyhow!("raid not found"))?;

        let raid_user = db
            .get_user_by_id(raid.user_id)?
            .ok_or_else(|| anyhow::anyhow!("raid user not found"))?;

        let squad = if let Some(ref server_id) = raid.server_id {
            db.get_raid_group(server_id)?
        } else {
            Vec::new()
        };

        let snapshot_sizes = db.get_raid_snapshot_sizes(raid_id)?;

        Ok::<_, anyhow::Error>((raid, kills, squad, raid_user.username, snapshot_sizes))
    })
    .await
    .map_err(WebError::from)?
    .map_err(|_| WebError::NotFound)?;

    let tmpl = RaidDetailPageTemplate {
        user,
        flash,
        csrf_token,
        nav: NavContext::from_state(&state),
        raid,
        kills,
        squad,
        raid_username,
        snapshot_sizes,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn active_raids_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<Html> {
    require_auth(&req)?;

    let db = state.db.clone();

    let active_raids = web::block(move || {
        let db = db.lock();
        db.get_active_raids()
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = ActiveRaidsPartialTemplate { active_raids };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn recent_raids_partial(
    state: Data<AppState>,
    req: HttpRequest,
) -> actix_web::Result<Html> {
    require_auth(&req)?;

    let db = state.db.clone();

    let recent_raids = web::block(move || {
        let db = db.lock();
        db.get_recent_raids(10)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = RecentRaidsPartialTemplate { recent_raids };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
