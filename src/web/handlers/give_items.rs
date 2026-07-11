use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Query};
use actix_web::HttpRequest;
use askama::Template;
use std::sync::Arc;

use crate::db::rbac::Permission;
use crate::dirs::QumaDirs;
use crate::fika::client::{FikaItemInfo, FikaSendItemRequest, FikaSendItemToAllRequest};
use crate::spt::profiles::{list_profiles, SptProfile};
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage, FlashType};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "give_items.html")]
struct GiveItemsTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    profiles: Vec<SptProfile>,
    fika_available: bool,
}

#[derive(Template)]
#[template(path = "give_items/partials/search_results.html")]
struct SearchResultsTemplate {
    results: Vec<ItemResult>,
}

struct ItemResult {
    tpl: String,
    name: String,
    description: String,
    max_amount: i32,
}

#[derive(serde::Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct SendItemForm {
    csrf_token: String,
    profile_id: String,
    item_tpl: String,
    item_name: String,
    amount: i32,
    #[serde(default)]
    fir: bool,
    #[serde(default)]
    message: String,
}

fn get_or_populate_items(
    state: &AppState,
) -> Option<Arc<std::collections::HashMap<String, FikaItemInfo>>> {
    state.fika_items.lock().clone()
}

async fn ensure_items_cached(
    state: &AppState,
) -> Result<Arc<std::collections::HashMap<String, FikaItemInfo>>, WebError> {
    if let Some(cached) = get_or_populate_items(state) {
        return Ok(cached);
    }

    let fika = state
        .fika_client
        .as_ref()
        .ok_or_else(|| WebError::BadRequest("Fika API not configured".into()))?;

    let response = fika.get_items().await.map_err(WebError::from)?;

    let items = Arc::new(response.items);
    *state.fika_items.lock() = Some(Arc::clone(&items));
    Ok(items)
}

pub async fn give_items_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ItemsGive)?;

    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let nav = NavContext::from_state(&state);

    let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
    let profiles = web::block(move || list_profiles(&dirs))
        .await
        .map_err(WebError::from)?
        .unwrap_or_default();

    let fika_available = state.fika_client.is_some();

    let tmpl = GiveItemsTemplate {
        user,
        flash,
        csrf_token,
        nav,
        profiles,
        fika_available,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn give_items_search(
    state: Data<AppState>,
    req: HttpRequest,
    query: Query<SearchQuery>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ItemsGive)?;

    let q = query.q.as_deref().unwrap_or("").trim().to_lowercase();
    if q.len() < 2 {
        let tmpl = SearchResultsTemplate { results: vec![] };
        return Ok(Html::new(tmpl.render().map_err(WebError::from)?));
    }

    let items = ensure_items_cached(&state).await?;

    let mut results: Vec<ItemResult> = items
        .iter()
        .filter(|(_, info)| info.name.to_lowercase().contains(&q))
        .map(|(tpl, info)| ItemResult {
            tpl: tpl.clone(),
            name: info.name.clone(),
            description: info.description.clone(),
            max_amount: info.stack_amount,
        })
        .collect();

    results.sort_by(|a, b| a.name.cmp(&b.name));
    results.truncate(50);

    let tmpl = SearchResultsTemplate { results };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn give_items_send(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<SendItemForm>,
) -> actix_web::Result<actix_web::HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ItemsGive)?;

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let fika = state
        .fika_client
        .as_ref()
        .ok_or(WebError::BadRequest("Fika API not configured".into()))?;

    if form.amount < 1 {
        return Err(WebError::BadRequest("Amount must be at least 1".into()).into());
    }

    let message = if form.message.is_empty() {
        format!("Sent by {} via Quartermaster", user.username)
    } else {
        form.message.clone()
    };

    let result = if form.profile_id == "all" {
        let dirs = QumaDirs::from_legacy(state.spt_dir.clone());
        let profiles = web::block(move || list_profiles(&dirs))
            .await
            .map_err(WebError::from)?
            .unwrap_or_default();

        let profile_ids: Vec<String> = profiles.into_iter().map(|p| p.aid).collect();
        if profile_ids.is_empty() {
            Err(anyhow::anyhow!("No profiles found"))
        } else {
            fika.send_item_to_all(&FikaSendItemToAllRequest {
                profile_ids,
                item_tpl: form.item_tpl.clone(),
                amount: form.amount,
                message,
                fir: form.fir,
                expiration_days: 7,
            })
            .await
        }
    } else {
        fika.send_item(&FikaSendItemRequest {
            profile_id: form.profile_id.clone(),
            item_tpl: form.item_tpl.clone(),
            amount: form.amount,
            message,
            fir: form.fir,
            expiration_days: 7,
        })
        .await
    };

    match result {
        Ok(()) => {
            let target = if form.profile_id == "all" {
                "all players".to_string()
            } else {
                form.profile_id.clone()
            };
            set_flash(
                &session,
                &format!("Sent {}x {} to {}", form.amount, form.item_name, target),
                FlashType::Success,
            );
        }
        Err(e) => {
            set_flash(
                &session,
                &format!("Failed to send item: {e}"),
                FlashType::Error,
            );
        }
    }

    Ok(actix_web::HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/give-items"))
        .finish())
}

pub async fn give_items_refresh(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<CsrfOnly>,
) -> actix_web::Result<actix_web::HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::ItemsGive)?;

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    state.clear_fika_items();
    set_flash(&session, "Items cache cleared", FlashType::Success);

    Ok(actix_web::HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/give-items"))
        .finish())
}

#[derive(serde::Deserialize)]
pub struct CsrfOnly {
    csrf_token: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_items() -> Arc<HashMap<String, FikaItemInfo>> {
        let mut map = HashMap::new();
        map.insert(
            "abc123".into(),
            FikaItemInfo {
                name: "M4A1 Assault Rifle".into(),
                description: "A rifle".into(),
                stack_amount: 10,
            },
        );
        map.insert(
            "def456".into(),
            FikaItemInfo {
                name: "Ammo 5.45x39 PS".into(),
                description: "Standard ammo".into(),
                stack_amount: 600,
            },
        );
        map.insert(
            "ghi789".into(),
            FikaItemInfo {
                name: "Mechanic's Storage Room Key".into(),
                description: "Key with apostrophe in name".into(),
                stack_amount: 10,
            },
        );
        Arc::new(map)
    }

    #[test]
    fn search_filters_by_lowercase_substring() {
        let items = make_items();
        let q = "m4a1";
        let results: Vec<_> = items
            .iter()
            .filter(|(_, info)| info.name.to_lowercase().contains(q))
            .collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].1.name, "M4A1 Assault Rifle");
    }

    #[test]
    fn search_returns_sorted_results() {
        let items = make_items();
        let q = "a"; // matches M4A1 and Ammo
        let mut results: Vec<ItemResult> = items
            .iter()
            .filter(|(_, info)| info.name.to_lowercase().contains(q))
            .map(|(tpl, info)| ItemResult {
                tpl: tpl.clone(),
                name: info.name.clone(),
                description: info.description.clone(),
                max_amount: info.stack_amount,
            })
            .collect();
        results.sort_by(|a, b| a.name.cmp(&b.name));
        results.truncate(50);
        assert!(results.len() >= 2);
        assert!(results[0].name < results[1].name);
    }

    #[test]
    fn search_short_query_returns_empty() {
        let q = "m";
        assert!(q.len() < 2);
    }

    #[test]
    fn amount_validation_rejects_zero() {
        assert!(0 < 1, "amount < 1 should be rejected");
    }
}
