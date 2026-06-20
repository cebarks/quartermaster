use actix_session::Session;
use actix_web::web::{self, Data, Html, Path, Query};
use actix_web::HttpRequest;
use askama::Template;
use serde::Deserialize;

use crate::spt::profiles::{load_profile_detail, ProfileDetail, QuestState};
use crate::web::auth::require_auth;
use crate::web::csrf;
use crate::web::error::WebError;
use crate::web::flash::{take_flash, FlashMessage};
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

// Display-ready structs — all lookups pre-resolved in handlers, not templates

struct QuestDisplay {
    name: String,
    status_label: String,
    status_css: String,
}

struct TraderDisplay {
    name: String,
    loyalty_level: i32,
    standing: f64,
    sales_sum: f64,
    currency: String,
}

struct HideoutAreaDisplay {
    name: String,
    level: i32,
    max_level: i32,
}

#[derive(Template)]
#[template(path = "profiles/page.html")]
struct ProfilePageTemplate {
    user: crate::web::auth::SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    fika_installed: bool,
    profile_username: String,
    detail: Option<ProfileDetail>,
    empty_reason: Option<String>,
    quests_html: String,
}

#[derive(Template)]
#[template(path = "profiles/partials/quests.html")]
struct QuestsPartialTemplate {
    quests: Vec<QuestDisplay>,
    total_quests: usize,
    completed_quests: usize,
    status_filter: String,
    search_filter: String,
    profile_username: String,
}

#[derive(Template)]
#[template(path = "profiles/partials/traders.html")]
struct TradersPartialTemplate {
    traders: Vec<TraderDisplay>,
}

#[derive(Template)]
#[template(path = "profiles/partials/hideout.html")]
struct HideoutPartialTemplate {
    areas: Vec<HideoutAreaDisplay>,
    total_current: i32,
    total_max: i32,
}

pub async fn profile_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<String>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = csrf::get_or_create_token(&session);
    let profile_username = path.into_inner();

    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let lookup_username = profile_username.clone();

    let (user_found, spt_profile_id) = web::block(move || {
        let db = db.lock();
        let target_user = db.get_user_by_username(&lookup_username)?;
        match target_user {
            None => Ok::<_, anyhow::Error>((false, None)),
            Some(u) => Ok((true, u.spt_profile_id.filter(|s| !s.is_empty()))),
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    if !user_found {
        return Err(WebError::NotFound.into());
    }

    let (detail, empty_reason) = match spt_profile_id {
        None => (
            None,
            Some("No SPT profile linked to this account.".to_string()),
        ),
        Some(profile_id) => {
            let spt_dir2 = spt_dir.clone();
            match web::block(move || load_profile_detail(&spt_dir2, &profile_id)).await {
                Ok(Ok(Some(d))) => (Some(d), None),
                Ok(Ok(None)) => (
                    None,
                    Some("This player hasn't completed their first raid yet.".to_string()),
                ),
                Ok(Err(_)) | Err(_) => (None, Some("Profile data unavailable.".to_string())),
            }
        }
    };

    let quests_html = if let Some(ref d) = detail {
        let total = d.quests.len();
        let completed = d
            .quests
            .iter()
            .filter(|q| matches!(q.status, QuestState::Success))
            .count();
        let quest_displays: Vec<QuestDisplay> = d
            .quests
            .iter()
            .map(|q| QuestDisplay {
                name: state.game_data.quest_name(&q.qid).to_string(),
                status_label: q.status.label().to_string(),
                status_css: q.status.css_class().to_string(),
            })
            .collect();
        let tmpl = QuestsPartialTemplate {
            quests: quest_displays,
            total_quests: total,
            completed_quests: completed,
            status_filter: String::new(),
            search_filter: String::new(),
            profile_username: profile_username.clone(),
        };
        tmpl.render().map_err(WebError::from)?
    } else {
        String::new()
    };

    let tmpl = ProfilePageTemplate {
        user,
        flash,
        csrf_token,
        fika_installed: state.fika_installed,
        profile_username,
        detail,
        empty_reason,
        quests_html,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

#[derive(Deserialize)]
pub struct QuestsQuery {
    #[serde(default)]
    status: String,
    #[serde(default)]
    search: String,
}

pub async fn quests_partial(
    state: Data<AppState>,
    req: HttpRequest,
    path: Path<String>,
    query: Query<QuestsQuery>,
) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let profile_username = path.into_inner();
    let detail = load_detail_for_user(&state, &profile_username).await?;

    let total = detail.quests.len();
    let completed = detail
        .quests
        .iter()
        .filter(|q| matches!(q.status, QuestState::Success))
        .count();

    let quest_displays: Vec<QuestDisplay> = detail
        .quests
        .iter()
        .filter(|q| {
            let status_ok = match query.status.as_str() {
                "completed" => matches!(q.status, QuestState::Success),
                "started" => matches!(
                    q.status,
                    QuestState::Started | QuestState::AvailableForFinish
                ),
                "available" => matches!(q.status, QuestState::AvailableForStart),
                "failed" => matches!(q.status, QuestState::Fail),
                "locked" => matches!(q.status, QuestState::Locked),
                _ => true,
            };
            let name = state.game_data.quest_name(&q.qid);
            let search_ok = query.search.is_empty()
                || name.to_lowercase().contains(&query.search.to_lowercase());
            status_ok && search_ok
        })
        .map(|q| QuestDisplay {
            name: state.game_data.quest_name(&q.qid).to_string(),
            status_label: q.status.label().to_string(),
            status_css: q.status.css_class().to_string(),
        })
        .collect();

    let tmpl = QuestsPartialTemplate {
        quests: quest_displays,
        total_quests: total,
        completed_quests: completed,
        status_filter: query.status.clone(),
        search_filter: query.search.clone(),
        profile_username,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn traders_partial(
    state: Data<AppState>,
    req: HttpRequest,
    path: Path<String>,
) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let profile_username = path.into_inner();
    let detail = load_detail_for_user(&state, &profile_username).await?;

    let trader_displays: Vec<TraderDisplay> = detail
        .traders
        .iter()
        .filter_map(|t| {
            state
                .game_data
                .trader_meta(&t.trader_id)
                .map(|meta| TraderDisplay {
                    name: meta.name.clone(),
                    loyalty_level: t.loyalty_level,
                    standing: t.standing,
                    sales_sum: t.sales_sum,
                    currency: meta.currency.clone(),
                })
        })
        .collect();

    let tmpl = TradersPartialTemplate {
        traders: trader_displays,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn hideout_partial(
    state: Data<AppState>,
    req: HttpRequest,
    path: Path<String>,
) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let profile_username = path.into_inner();
    let detail = load_detail_for_user(&state, &profile_username).await?;

    let area_displays: Vec<HideoutAreaDisplay> = detail
        .hideout
        .iter()
        .map(|h| {
            let meta = state.game_data.hideout_area(h.area_type);
            HideoutAreaDisplay {
                name: meta
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| format!("Area {}", h.area_type)),
                level: h.level,
                max_level: meta.map(|m| m.max_level).unwrap_or(0),
            }
        })
        .collect();

    let total_current: i32 = area_displays.iter().map(|a| a.level).sum();
    let total_max: i32 = area_displays.iter().map(|a| a.max_level).sum();

    let tmpl = HideoutPartialTemplate {
        areas: area_displays,
        total_current,
        total_max,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

async fn load_detail_for_user(
    state: &Data<AppState>,
    username: &str,
) -> Result<ProfileDetail, WebError> {
    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let username = username.to_string();

    let spt_profile_id = web::block(move || {
        let db = db.lock();
        let target_user = db.get_user_by_username(&username)?;
        match target_user {
            None => Err(anyhow::anyhow!("user not found")),
            Some(u) => Ok(u.spt_profile_id.filter(|s| !s.is_empty())),
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(|_| WebError::NotFound)?;

    let profile_id = spt_profile_id.ok_or(WebError::NotFound)?;

    let detail = web::block(move || load_profile_detail(&spt_dir, &profile_id))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    detail.ok_or(WebError::NotFound)
}
