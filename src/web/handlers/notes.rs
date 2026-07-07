use actix_session::Session;
use actix_web::web::{self, Data, Form, Html, Path};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;

use crate::db::notes::NoteView;
use crate::db::rbac::Permission;
use crate::web::auth::{require_auth, SessionUser};
use crate::web::csrf;
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage, FlashType};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[allow(unused_imports)]
mod filters {
    pub use crate::web::template_filters::*;
}

#[derive(serde::Deserialize)]
pub struct NoteForm {
    pub title: String,
    pub content: String,
    pub visibility: String,
    pub pinned: Option<String>,
    pub csrf_token: String,
}

#[derive(Template)]
#[template(path = "notes.html")]
struct NotesTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    my_notes: Vec<NoteView>,
    shared_notes: Vec<NoteView>,
}

#[derive(Template)]
#[template(path = "notes_edit.html")]
struct NotesEditTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    note: Option<NoteView>,
}

// ponytail: helpers used in tests and edit_note_form
fn can_edit_note(note: &NoteView, user: &SessionUser) -> bool {
    note.author_id == user.user_id
        || (note.visibility == "public_editable" && user.has_permission(Permission::NotesEdit))
}

#[cfg(test)]
fn can_delete_note(note: &NoteView, user: &SessionUser) -> bool {
    note.author_id == user.user_id || user.has_permission(Permission::NotesManage)
}

fn validate_visibility(s: &str) -> Result<&str, WebError> {
    match s {
        "private" | "public_readonly" | "public_editable" => Ok(s),
        _ => Err(WebError::BadRequest("Invalid visibility.".into())),
    }
}

pub async fn notes_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = csrf::get_or_create_token(&session);
    let nav = NavContext::from_state(&state);

    let db = state.db.clone();
    let user_id = user.user_id;
    let has_manage = user.has_permission(Permission::NotesManage);
    let (my_notes, shared_notes) = web::block(move || {
        let db = db.lock();
        let my = db.list_notes_for_user(user_id)?;
        let shared = if has_manage {
            db.list_other_notes(user_id)?
        } else {
            db.list_public_notes(user_id)?
        };
        Ok::<_, rusqlite::Error>((my, shared))
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    let tmpl = NotesTemplate {
        user,
        flash,
        csrf_token,
        nav,
        my_notes,
        shared_notes,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn new_note_form(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = csrf::get_or_create_token(&session);
    let nav = NavContext::from_state(&state);

    let tmpl = NotesEditTemplate {
        user,
        flash,
        csrf_token,
        nav,
        note: None,
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn create_note(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<NoteForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let title = form.title.trim().to_string();
    let content = form.content.trim().to_string();
    if title.is_empty() || content.is_empty() {
        return Err(WebError::BadRequest("Title and content are required.".into()).into());
    }
    let visibility = validate_visibility(&form.visibility)?;
    let pinned = form.pinned.is_some();

    let db = state.db.clone();
    let user_id = user.user_id;
    let visibility = visibility.to_string();
    web::block(move || {
        let db = db.lock();
        db.create_note(user_id, &title, &content, &visibility, pinned)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?;

    set_flash(&session, "Note created.", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/notes"))
        .finish())
}

pub async fn edit_note_form(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
) -> actix_web::Result<Html> {
    let user = require_auth(&req)?;
    let flash = take_flash(&session);
    let csrf_token = csrf::get_or_create_token(&session);
    let nav = NavContext::from_state(&state);
    let note_id = path.into_inner();

    let db = state.db.clone();
    let note = web::block(move || {
        let db = db.lock();
        db.get_note(note_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(WebError::from)?
    .ok_or(WebError::NotFound)?;

    if !can_edit_note(&note, &user) {
        return Err(WebError::Forbidden.into());
    }

    let tmpl = NotesEditTemplate {
        user,
        flash,
        csrf_token,
        nav,
        note: Some(note),
    };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn update_note(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<NoteForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let note_id = path.into_inner();

    let title = form.title.trim().to_string();
    let content = form.content.trim().to_string();
    if title.is_empty() || content.is_empty() {
        return Err(WebError::BadRequest("Title and content are required.".into()).into());
    }
    let form_visibility = validate_visibility(&form.visibility)?.to_string();
    let form_pinned = form.pinned.is_some();

    let db = state.db.clone();
    let user_id = user.user_id;
    let has_edit = user.has_permission(Permission::NotesEdit);
    let updated = web::block(move || {
        let db = db.lock();
        let note = db
            .get_note(note_id)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;

        let is_author = note.author_id == user_id;
        let can_edit = is_author || (note.visibility == "public_editable" && has_edit);
        if !can_edit {
            return Ok(false);
        }

        let (visibility, pinned) = if is_author {
            (form_visibility, form_pinned)
        } else {
            (note.visibility, note.pinned)
        };

        db.update_note(note_id, &title, &content, &visibility, pinned, user_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(|e| {
        if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
            WebError::NotFound
        } else {
            WebError::from(e)
        }
    })?;

    if !updated {
        return Err(WebError::Forbidden.into());
    }

    set_flash(&session, "Note updated.", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/notes"))
        .finish())
}

pub async fn delete_note(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    path: Path<i64>,
    form: Form<csrf::CsrfForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    if !csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }
    let note_id = path.into_inner();

    let db = state.db.clone();
    let user_id = user.user_id;
    let has_manage = user.has_permission(Permission::NotesManage);
    let deleted = web::block(move || {
        let db = db.lock();
        let note = db
            .get_note(note_id)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;

        if note.author_id != user_id && !has_manage {
            return Ok(false);
        }

        db.delete_note(note_id)
    })
    .await
    .map_err(WebError::from)?
    .map_err(|e| {
        if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
            WebError::NotFound
        } else {
            WebError::from(e)
        }
    })?;

    if !deleted {
        return Err(WebError::Forbidden.into());
    }

    set_flash(&session, "Note deleted.", FlashType::Success);
    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/notes"))
        .finish())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn make_user(id: i64, role: &str, perms: &[Permission]) -> SessionUser {
        SessionUser {
            user_id: id,
            username: "test".into(),
            role_name: role.into(),
            role_display_name: role.into(),
            permissions: perms.iter().copied().collect(),
            has_password: true,
        }
    }

    fn make_note(author_id: i64, visibility: &str) -> NoteView {
        NoteView {
            id: 1,
            author_id,
            author_username: "author".into(),
            title: "t".into(),
            content: "c".into(),
            visibility: visibility.into(),
            pinned: false,
            created_at: "".into(),
            updated_at: "".into(),
            updated_by: None,
            updated_by_username: None,
        }
    }

    #[test]
    fn author_can_edit_own_private_note() {
        let user = make_user(1, "player", &[]);
        let note = make_note(1, "private");
        assert!(can_edit_note(&note, &user));
    }

    #[test]
    fn non_author_cannot_edit_private_note() {
        let user = make_user(2, "player", &[]);
        let note = make_note(1, "private");
        assert!(!can_edit_note(&note, &user));
    }

    #[test]
    fn non_author_cannot_edit_public_readonly() {
        let user = make_user(2, "player", &[Permission::NotesEdit]);
        let note = make_note(1, "public_readonly");
        assert!(!can_edit_note(&note, &user));
    }

    #[test]
    fn notes_edit_allows_editing_public_editable() {
        let user = make_user(2, "moderator", &[Permission::NotesEdit]);
        let note = make_note(1, "public_editable");
        assert!(can_edit_note(&note, &user));
    }

    #[test]
    fn notes_edit_without_permission_cannot_edit() {
        let user = make_user(2, "player", &[]);
        let note = make_note(1, "public_editable");
        assert!(!can_edit_note(&note, &user));
    }

    #[test]
    fn author_can_delete_own_note() {
        let user = make_user(1, "player", &[]);
        let note = make_note(1, "private");
        assert!(can_delete_note(&note, &user));
    }

    #[test]
    fn notes_manage_can_delete_others() {
        let user = make_user(2, "admin", &[Permission::NotesManage]);
        let note = make_note(1, "private");
        assert!(can_delete_note(&note, &user));
    }

    #[test]
    fn notes_edit_cannot_delete_others() {
        let user = make_user(2, "moderator", &[Permission::NotesEdit]);
        let note = make_note(1, "public_editable");
        assert!(!can_delete_note(&note, &user));
    }

    #[test]
    fn validate_visibility_accepts_valid() {
        assert!(validate_visibility("private").is_ok());
        assert!(validate_visibility("public_readonly").is_ok());
        assert!(validate_visibility("public_editable").is_ok());
    }

    #[test]
    fn validate_visibility_rejects_invalid() {
        assert!(validate_visibility("invalid").is_err());
        assert!(validate_visibility("").is_err());
    }
}
