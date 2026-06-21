use actix_session::Session;
use actix_web::web::{self, Data, Html, Path, Query};
use actix_web::HttpRequest;
use askama::Template;
use serde::Deserialize;

use crate::spt::profiles::{load_profile_detail, load_stash_items, ProfileDetail, QuestState};
use crate::web::auth::require_auth;
use crate::web::csrf;
use crate::web::error::WebError;
use crate::web::flash::{take_flash, FlashMessage};
use crate::web::nav::NavContext;
use crate::web::state::AppState;
use std::collections::HashMap;

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

struct StashItemDisplay {
    name: String,
    short_name: String,
    count: i64,
    total_value: i64,
}

struct StashCategoryDisplay {
    name: String,
    items: Vec<StashItemDisplay>,
    total_value: i64,
    item_count: usize,
}

#[derive(Template)]
#[template(path = "profiles/page.html")]
struct ProfilePageTemplate {
    user: crate::web::auth::SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    profile_username: String,
    detail: Option<ProfileDetail>,
    empty_reason: Option<String>,
    quests_html: String,
    registration_date: Option<String>,
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

#[derive(Template)]
#[template(path = "profiles/partials/stash_visibility.html")]
struct StashVisibilityTemplate {
    csrf_token: String,
    profile_username: String,
    stash_public: bool,
}

#[derive(Template)]
#[template(path = "profiles/partials/stash.html")]
struct StashPartialTemplate {
    categories: Vec<StashCategoryDisplay>,
    total_items: usize,
    total_value: i64,
    search_filter: String,
    category_filter: String,
    all_categories: Vec<String>,
    profile_username: String,
    is_own_profile: bool,
    stash_public: bool,
    can_view: bool,
    csrf_token: String,
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
            let game_data = state.game_data.clone();
            match web::block(move || {
                load_profile_detail(&spt_dir2, &profile_id, game_data.prices())
            })
            .await
            {
                Ok(Ok(Some(d))) => (Some(d), None),
                Ok(Ok(None)) => (
                    None,
                    Some("This player hasn't completed their first raid yet.".to_string()),
                ),
                Ok(Err(_)) | Err(_) => (None, Some("Profile data unavailable.".to_string())),
            }
        }
    };

    let registration_date = detail.as_ref().and_then(|d| {
        d.stats.registration_date.and_then(|ts| {
            chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.format("%Y-%m-%d").to_string())
        })
    });

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
        nav: NavContext::from_state(&state),
        profile_username,
        detail,
        empty_reason,
        quests_html,
        registration_date,
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

#[derive(Deserialize)]
pub struct StashQuery {
    #[serde(default)]
    search: String,
    #[serde(default)]
    category: String,
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

    let game_data = state.game_data.clone();
    let detail = web::block(move || load_profile_detail(&spt_dir, &profile_id, game_data.prices()))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    detail.ok_or(WebError::NotFound)
}

pub async fn stash_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<String>,
    query: Query<StashQuery>,
) -> actix_web::Result<Html> {
    let viewer = require_auth(&req)?;
    let csrf_token = csrf::get_or_create_token(&session);
    let profile_username = path.into_inner();

    let db = state.db.clone();
    let lookup_username = profile_username.clone();
    let (target_user_id, spt_profile_id, stash_public) = web::block(move || {
        let db = db.lock();
        let target_user = db.get_user_by_username(&lookup_username)?;
        match target_user {
            None => Err(anyhow::anyhow!("user not found")),
            Some(u) => Ok((
                u.id,
                u.spt_profile_id.filter(|s| !s.is_empty()),
                u.stash_public,
            )),
        }
    })
    .await
    .map_err(WebError::from)?
    .map_err(|_| WebError::NotFound)?;

    let is_own_profile = viewer.user_id == target_user_id;
    let is_admin = viewer.role == crate::db::users::Role::Admin;
    let can_view = is_own_profile || is_admin || stash_public;

    if !can_view {
        let tmpl = StashPartialTemplate {
            categories: Vec::new(),
            total_items: 0,
            total_value: 0,
            search_filter: String::new(),
            category_filter: String::new(),
            all_categories: Vec::new(),
            profile_username,
            is_own_profile,
            stash_public,
            can_view: false,
            csrf_token: csrf_token.clone(),
        };
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    let profile_id = match spt_profile_id {
        Some(id) => id,
        None => {
            let tmpl = StashPartialTemplate {
                categories: Vec::new(),
                total_items: 0,
                total_value: 0,
                search_filter: String::new(),
                category_filter: String::new(),
                all_categories: Vec::new(),
                profile_username,
                is_own_profile,
                stash_public,
                can_view: true,
                csrf_token: csrf_token.clone(),
            };
            return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
        }
    };

    let spt_dir = state.spt_dir.clone();
    let raw_items = web::block(move || load_stash_items(&spt_dir, &profile_id))
        .await
        .map_err(WebError::from)?
        .map_err(WebError::from)?;

    let raw_items = match raw_items {
        Some(items) => items,
        None => {
            let tmpl = StashPartialTemplate {
                categories: Vec::new(),
                total_items: 0,
                total_value: 0,
                search_filter: String::new(),
                category_filter: String::new(),
                all_categories: Vec::new(),
                profile_username,
                is_own_profile,
                stash_public,
                can_view: true,
                csrf_token: csrf_token.clone(),
            };
            return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
        }
    };

    // Aggregate by template ID
    let mut aggregated: HashMap<String, i64> = HashMap::new();
    for item in &raw_items {
        *aggregated.entry(item.tpl.clone()).or_default() += item.count;
    }

    // Resolve display data and apply filters
    let search_lower = query.search.to_lowercase();
    let mut items_by_category: HashMap<String, Vec<StashItemDisplay>> = HashMap::new();
    let mut all_categories_set: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();

    for (tpl, count) in &aggregated {
        let name = state.game_data.item_name(tpl).to_string();
        let short_name = state.game_data.item_short_name(tpl).to_string();
        let category = state.game_data.item_category(tpl).to_string();
        let unit_price = state.game_data.item_price(tpl).unwrap_or(0);
        let total_value = unit_price.saturating_mul(*count);

        all_categories_set.insert(category.clone());

        // Apply search filter
        if !search_lower.is_empty()
            && !name.to_lowercase().contains(&search_lower)
            && !short_name.to_lowercase().contains(&search_lower)
        {
            continue;
        }

        // Apply category filter
        if !query.category.is_empty() && category != query.category {
            continue;
        }

        items_by_category
            .entry(category)
            .or_default()
            .push(StashItemDisplay {
                name,
                short_name,
                count: *count,
                total_value,
            });
    }

    // Sort items within each category by value descending
    for items in items_by_category.values_mut() {
        items.sort_by_key(|b| std::cmp::Reverse(b.total_value));
    }

    // Build sorted category list
    let mut categories: Vec<StashCategoryDisplay> = items_by_category
        .into_iter()
        .map(|(name, items)| {
            let total_value = items.iter().map(|i| i.total_value).sum();
            let item_count = items.len();
            StashCategoryDisplay {
                name,
                items,
                total_value,
                item_count,
            }
        })
        .collect();
    categories.sort_by(|a, b| a.name.cmp(&b.name));

    let total_items: usize = categories.iter().map(|c| c.item_count).sum();
    let total_value: i64 = categories.iter().map(|c| c.total_value).sum();
    let all_categories: Vec<String> = all_categories_set.into_iter().collect();

    let tmpl = StashPartialTemplate {
        categories,
        total_items,
        total_value,
        search_filter: query.search.clone(),
        category_filter: query.category.clone(),
        all_categories,
        profile_username,
        is_own_profile,
        stash_public,
        can_view: true,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn toggle_stash_visibility(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<String>,
    form: web::Form<csrf::CsrfForm>,
) -> actix_web::Result<Html> {
    let viewer = require_auth(&req)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let csrf_token = csrf::get_or_create_token(&session);
    let profile_username = path.into_inner();

    // Only allow toggling own profile
    if viewer.username != profile_username {
        return Err(WebError::Forbidden.into());
    }

    let db = state.db.clone();
    let user_id = viewer.user_id;
    let new_value = web::block(move || {
        let db = db.lock();
        let user = db
            .get_user_by_id(user_id)?
            .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)?;
        let new_val = !user.stash_public;
        db.set_stash_public(user_id, new_val)?;
        Ok::<bool, rusqlite::Error>(new_val)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = StashVisibilityTemplate {
        csrf_token,
        profile_username,
        stash_public: new_value,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
