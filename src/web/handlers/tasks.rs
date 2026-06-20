use actix_session::Session;
use actix_web::web::{Data, Form, Html, Path};
use actix_web::HttpRequest;
use askama::Template;

use crate::web::auth::require_auth;
use crate::web::error::WebError;
use crate::web::state::AppState;
use crate::web::tasks::TaskView;

#[derive(Template)]
#[template(path = "partials/task_status.html")]
struct TaskStatusTemplate {
    tasks: Vec<TaskView>,
    csrf_token: String,
}

pub async fn task_status_partial(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    require_auth(&req)?;
    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let tasks = state.tasks.task_views();
    let tmpl = TaskStatusTemplate { tasks, csrf_token };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn dismiss_task(
    state: Data<AppState>,
    path: Path<u64>,
    req: HttpRequest,
    session: Session,
    form: Form<crate::web::csrf::CsrfForm>,
) -> actix_web::Result<Html> {
    require_auth(&req)?;
    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    state.tasks.dismiss(path.into_inner());

    let csrf_token = crate::web::csrf::get_or_create_token(&session);
    let tasks = state.tasks.task_views();
    let tmpl = TaskStatusTemplate { tasks, csrf_token };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}
