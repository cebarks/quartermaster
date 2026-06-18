use actix_session::Session;
use actix_web::web::{Data, Form, Html, Path};
use actix_web::HttpResponse;
use askama::Template;

use crate::web::auth::require_auth;
use crate::web::error::WebError;
use crate::web::state::AppState;
use crate::web::tasks::TaskView;

#[derive(Template)]
#[template(path = "partials/task_status.html")]
struct TaskStatusTemplate {
    tasks: Vec<TaskView>,
    has_active: bool,
    csrf_token: String,
}

pub async fn task_status_partial(
    state: Data<AppState>,
    session: Session,
) -> actix_web::Result<Html> {
    require_auth(&session)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let tasks = state.tasks.task_views();
    let has_active = state.tasks.has_active();
    let tmpl = TaskStatusTemplate {
        tasks,
        has_active,
        csrf_token,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn dismiss_task(
    state: Data<AppState>,
    path: Path<u64>,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    require_auth(&session)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    state.tasks.dismiss(path.into_inner());
    Ok(HttpResponse::Ok().body(""))
}
