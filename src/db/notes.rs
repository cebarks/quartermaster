#![allow(dead_code)] // ponytail: used by web handlers in Task 3

use rusqlite::{params, OptionalExtension};

use super::Database;

#[derive(Debug, Clone)]
pub struct NoteView {
    pub id: i64,
    pub author_id: i64,
    pub author_username: String,
    pub title: String,
    pub content: String,
    pub visibility: String,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    pub updated_by: Option<i64>,
    pub updated_by_username: Option<String>,
}

fn row_to_note_view(row: &rusqlite::Row) -> rusqlite::Result<NoteView> {
    Ok(NoteView {
        id: row.get(0)?,
        author_id: row.get(1)?,
        author_username: row.get(2)?,
        title: row.get(3)?,
        content: row.get(4)?,
        visibility: row.get(5)?,
        pinned: row.get::<_, i32>(6)? != 0,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        updated_by: row.get(9)?,
        updated_by_username: row.get(10)?,
    })
}

impl Database {
    pub fn create_note(
        &self,
        author_id: i64,
        title: &str,
        content: &str,
        visibility: &str,
        pinned: bool,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO notes (author_id, title, content, visibility, pinned)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![author_id, title, content, visibility, pinned as i32],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_note(
        &self,
        id: i64,
        title: &str,
        content: &str,
        visibility: &str,
        pinned: bool,
        updated_by: i64,
    ) -> rusqlite::Result<bool> {
        let rows = self.conn.execute(
            "UPDATE notes SET title = ?1, content = ?2, visibility = ?3, pinned = ?4,
             updated_by = ?5, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE id = ?6",
            params![title, content, visibility, pinned as i32, updated_by, id],
        )?;
        Ok(rows > 0)
    }

    pub fn delete_note(&self, id: i64) -> rusqlite::Result<bool> {
        let rows = self
            .conn
            .execute("DELETE FROM notes WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    pub fn get_note(&self, id: i64) -> rusqlite::Result<Option<NoteView>> {
        self.conn
            .query_row(
                "SELECT n.id, n.author_id, u.username, n.title, n.content,
                        n.visibility, n.pinned, n.created_at, n.updated_at,
                        n.updated_by, ub.username
                 FROM notes n
                 JOIN users u ON n.author_id = u.id
                 LEFT JOIN users ub ON n.updated_by = ub.id
                 WHERE n.id = ?1",
                params![id],
                row_to_note_view,
            )
            .optional()
    }

    pub fn list_notes_for_user(&self, user_id: i64) -> rusqlite::Result<Vec<NoteView>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.author_id, u.username, n.title, n.content,
                    n.visibility, n.pinned, n.created_at, n.updated_at,
                    n.updated_by, ub.username
             FROM notes n
             JOIN users u ON n.author_id = u.id
             LEFT JOIN users ub ON n.updated_by = ub.id
             WHERE n.author_id = ?1
             ORDER BY n.pinned DESC, n.created_at DESC",
        )?;
        let rows = stmt.query_map(params![user_id], row_to_note_view)?;
        rows.collect()
    }

    pub fn list_public_notes(&self, exclude_author_id: i64) -> rusqlite::Result<Vec<NoteView>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.author_id, u.username, n.title, n.content,
                    n.visibility, n.pinned, n.created_at, n.updated_at,
                    n.updated_by, ub.username
             FROM notes n
             JOIN users u ON n.author_id = u.id
             LEFT JOIN users ub ON n.updated_by = ub.id
             WHERE n.author_id != ?1
               AND n.visibility IN ('public_readonly', 'public_editable')
             ORDER BY n.pinned DESC, n.created_at DESC",
        )?;
        let rows = stmt.query_map(params![exclude_author_id], row_to_note_view)?;
        rows.collect()
    }

    pub fn list_other_notes(&self, exclude_author_id: i64) -> rusqlite::Result<Vec<NoteView>> {
        let mut stmt = self.conn.prepare(
            "SELECT n.id, n.author_id, u.username, n.title, n.content,
                    n.visibility, n.pinned, n.created_at, n.updated_at,
                    n.updated_by, ub.username
             FROM notes n
             JOIN users u ON n.author_id = u.id
             LEFT JOIN users ub ON n.updated_by = ub.id
             WHERE n.author_id != ?1
             ORDER BY n.pinned DESC, n.created_at DESC",
        )?;
        let rows = stmt.query_map(params![exclude_author_id], row_to_note_view)?;
        rows.collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn setup_db_with_users() -> (Database, i64, i64) {
        let db = Database::open_in_memory().unwrap();
        let alice = db
            .insert_user("alice", None, Some("hash"), "admin", false)
            .unwrap();
        let bob = db
            .insert_user("bob", None, Some("hash"), "player", false)
            .unwrap();
        (db, alice, bob)
    }

    #[test]
    fn create_and_get_note() {
        let (db, alice, _) = setup_db_with_users();
        let id = db
            .create_note(alice, "Test Note", "# Hello", "private", false)
            .unwrap();
        let note = db.get_note(id).unwrap().unwrap();
        assert_eq!(note.title, "Test Note");
        assert_eq!(note.content, "# Hello");
        assert_eq!(note.visibility, "private");
        assert_eq!(note.author_username, "alice");
        assert!(!note.pinned);
    }

    #[test]
    fn update_note() {
        let (db, alice, bob) = setup_db_with_users();
        let id = db
            .create_note(alice, "Original", "content", "private", false)
            .unwrap();
        let updated = db
            .update_note(id, "Updated", "new content", "public_readonly", true, bob)
            .unwrap();
        assert!(updated);
        let note = db.get_note(id).unwrap().unwrap();
        assert_eq!(note.title, "Updated");
        assert_eq!(note.content, "new content");
        assert_eq!(note.visibility, "public_readonly");
        assert!(note.pinned);
        assert_eq!(note.updated_by, Some(bob));
        assert_eq!(note.updated_by_username.as_deref(), Some("bob"));
    }

    #[test]
    fn delete_note() {
        let (db, alice, _) = setup_db_with_users();
        let id = db
            .create_note(alice, "Delete me", "content", "private", false)
            .unwrap();
        assert!(db.delete_note(id).unwrap());
        assert!(db.get_note(id).unwrap().is_none());
        assert!(!db.delete_note(id).unwrap());
    }

    #[test]
    fn list_notes_for_user() {
        let (db, alice, bob) = setup_db_with_users();
        db.create_note(alice, "Alice 1", "c", "private", false)
            .unwrap();
        db.create_note(alice, "Alice 2", "c", "public_readonly", true)
            .unwrap();
        db.create_note(bob, "Bob 1", "c", "private", false).unwrap();
        let notes = db.list_notes_for_user(alice).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].title, "Alice 2"); // pinned first
    }

    #[test]
    fn list_public_notes_excludes_private_and_self() {
        let (db, alice, bob) = setup_db_with_users();
        db.create_note(alice, "Private", "c", "private", false)
            .unwrap();
        db.create_note(alice, "Public", "c", "public_readonly", false)
            .unwrap();
        db.create_note(bob, "Bob Public", "c", "public_editable", false)
            .unwrap();
        let notes = db.list_public_notes(alice).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].title, "Bob Public");
    }

    #[test]
    fn list_other_notes_includes_private_from_others() {
        let (db, alice, bob) = setup_db_with_users();
        db.create_note(alice, "Alice Private", "c", "private", false)
            .unwrap();
        db.create_note(bob, "Bob Private", "c", "private", false)
            .unwrap();
        db.create_note(bob, "Bob Public", "c", "public_readonly", false)
            .unwrap();
        let notes = db.list_other_notes(alice).unwrap();
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().all(|n| n.author_id != alice));
    }
}
