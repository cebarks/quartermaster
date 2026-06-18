use actix_session::Session;
use actix_web::web::{self, Data, Html};
use askama::Template;

use crate::cli::common::find_unmanaged_mod_dirs;
use crate::db::mods::InstalledMod;
use crate::web::auth::{get_session_user, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    user: SessionUser,
    mods: Vec<InstalledMod>,
    pending_count: usize,
    unmanaged_dirs: Vec<(String, usize)>,
}

pub async fn dashboard(state: Data<AppState>, session: Session) -> actix_web::Result<Html> {
    let user = get_session_user(&session).ok_or(WebError::Forbidden)?;

    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();

    let (mods, pending_count, unmanaged_dirs) = web::block(move || {
        let db = db.lock();
        let mods = db.list_mods()?;
        let pending = db.list_pending_ops()?;
        let (dirs, _total) = find_unmanaged_mod_dirs(&spt_dir, &db)?;
        let dirs_vec: Vec<(String, usize)> = dirs.into_iter().collect();
        Ok::<_, anyhow::Error>((mods, pending.len(), dirs_vec))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = DashboardTemplate {
        user,
        mods,
        pending_count,
        unmanaged_dirs,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
