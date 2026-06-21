use actix_session::Session;
use actix_web::web::{self, Data, Html};
use actix_web::HttpRequest;
use askama::Template;

use crate::db::raids::LeaderboardEntry;
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
#[template(path = "leaderboard.html")]
struct LeaderboardPageTemplate {
    user: crate::web::auth::SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    entries: Vec<LeaderboardEntry>,
    min_raids: u32,
}

pub async fn leaderboard_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = csrf::get_or_create_token(&session);

    let db = state.db.clone();
    let min_raids = state.config.leaderboard_min_raids;

    let entries = web::block(move || {
        let db = db.lock();
        db.get_leaderboard(min_raids)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = LeaderboardPageTemplate {
        user,
        flash,
        csrf_token,
        nav: NavContext::from_state(&state),
        entries,
        min_raids,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
