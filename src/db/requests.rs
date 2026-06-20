use rusqlite::params;

use super::Database;

#[derive(Debug, Clone)]
pub struct ModRequest {
    pub id: i64,
    pub user_id: i64,
    pub forge_mod_id: i64,
    pub mod_name: String,
    pub mod_slug: Option<String>,
    pub mod_description: Option<String>,
    pub fika_compatible: String,
    pub reason: Option<String>,
    pub status: String,
    pub resolved_by: Option<i64>,
    pub resolved_at: Option<String>,
    pub resolve_comment: Option<String>,
    pub created_at: String,
    pub forge_cached_at: String,
}

#[derive(Debug, Clone)]
pub struct ModRequestVote {
    pub id: i64,
    pub request_id: i64,
    pub user_id: i64,
    pub upvote: bool,
    pub comment: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct ModRequestView {
    pub request: ModRequest,
    pub requester_username: String,
    pub vote_score: i64,
    pub upvote_count: i64,
    pub downvote_count: i64,
    pub comment_count: i64,
    pub current_user_vote: Option<bool>,
    pub resolver_username: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VoteComment {
    pub username: String,
    pub upvote: bool,
    pub comment: String,
    pub created_at: String,
}

impl Database {
    #[allow(clippy::too_many_arguments)]
    pub fn create_mod_request(
        &self,
        user_id: i64,
        forge_mod_id: i64,
        mod_name: &str,
        mod_slug: Option<&str>,
        mod_description: Option<&str>,
        fika_compatible: &str,
        reason: Option<&str>,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO mod_requests (user_id, forge_mod_id, mod_name, mod_slug, mod_description, fika_compatible, reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![user_id, forge_mod_id, mod_name, mod_slug, mod_description, fika_compatible, reason],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_mod_requests(
        &self,
        status: Option<&str>,
        current_user_id: i64,
    ) -> rusqlite::Result<Vec<ModRequestView>> {
        let base_query = "
            SELECT
                r.id, r.user_id, r.forge_mod_id, r.mod_name, r.mod_slug,
                r.mod_description, r.fika_compatible, r.reason, r.status,
                r.resolved_by, r.resolved_at, r.resolve_comment,
                r.created_at, r.forge_cached_at,
                u.username AS requester_username,
                COALESCE(SUM(CASE WHEN v.upvote = 1 THEN 1 ELSE 0 END), 0) AS upvote_count,
                COALESCE(SUM(CASE WHEN v.upvote = 0 THEN 1 ELSE 0 END), 0) AS downvote_count,
                COALESCE(SUM(CASE WHEN v.comment IS NOT NULL AND v.comment != '' THEN 1 ELSE 0 END), 0) AS comment_count,
                cv.upvote AS current_user_vote,
                ru.username AS resolver_username
            FROM mod_requests r
            JOIN users u ON r.user_id = u.id
            LEFT JOIN mod_request_votes v ON r.id = v.request_id
            LEFT JOIN mod_request_votes cv ON r.id = cv.request_id AND cv.user_id = ?1
            LEFT JOIN users ru ON r.resolved_by = ru.id
        ";

        let (query, do_filter) = match status {
            Some(s) if !s.is_empty() => (
                format!(
                    "{base_query} WHERE r.status = ?2
                     GROUP BY r.id
                     ORDER BY (COALESCE(SUM(CASE WHEN v.upvote = 1 THEN 1 ELSE 0 END), 0)
                             - COALESCE(SUM(CASE WHEN v.upvote = 0 THEN 1 ELSE 0 END), 0)) DESC,
                              r.created_at DESC"
                ),
                Some(s),
            ),
            _ => (
                format!(
                    "{base_query}
                     GROUP BY r.id
                     ORDER BY (COALESCE(SUM(CASE WHEN v.upvote = 1 THEN 1 ELSE 0 END), 0)
                             - COALESCE(SUM(CASE WHEN v.upvote = 0 THEN 1 ELSE 0 END), 0)) DESC,
                              r.created_at DESC"
                ),
                None,
            ),
        };

        let mut stmt = self.conn.prepare(&query)?;

        let rows = if let Some(s) = do_filter {
            stmt.query_map(params![current_user_id, s], row_to_request_view)?
        } else {
            stmt.query_map(params![current_user_id], row_to_request_view)?
        };

        rows.collect()
    }

    pub fn get_mod_request(&self, id: i64) -> rusqlite::Result<Option<ModRequest>> {
        self.conn
            .query_row(
                "SELECT id, user_id, forge_mod_id, mod_name, mod_slug, mod_description,
                        fika_compatible, reason, status, resolved_by, resolved_at,
                        resolve_comment, created_at, forge_cached_at
                 FROM mod_requests WHERE id = ?1",
                params![id],
                row_to_request,
            )
            .optional()
    }

    pub fn has_pending_request_for_mod(&self, forge_mod_id: i64) -> rusqlite::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM mod_requests WHERE forge_mod_id = ?1 AND status = 'pending'",
            params![forge_mod_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn resolve_mod_request(
        &self,
        id: i64,
        status: &str,
        resolved_by: i64,
        comment: Option<&str>,
    ) -> rusqlite::Result<usize> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE mod_requests SET status = ?1, resolved_by = ?2, resolved_at = ?3, resolve_comment = ?4
             WHERE id = ?5 AND status = 'pending'",
            params![status, resolved_by, now, comment, id],
        )
    }

    pub fn update_mod_request_cache(
        &self,
        id: i64,
        mod_name: &str,
        mod_slug: Option<&str>,
        mod_description: Option<&str>,
        fika_compatible: &str,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE mod_requests SET mod_name = ?1, mod_slug = ?2, mod_description = ?3,
                    fika_compatible = ?4, forge_cached_at = datetime('now')
             WHERE id = ?5",
            params![mod_name, mod_slug, mod_description, fika_compatible, id],
        )
    }

    pub fn upsert_vote(
        &self,
        request_id: i64,
        user_id: i64,
        upvote: bool,
        comment: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO mod_request_votes (request_id, user_id, upvote, comment)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(request_id, user_id) DO UPDATE SET upvote = ?3, comment = ?4",
            params![request_id, user_id, upvote as i32, comment],
        )?;
        Ok(())
    }

    pub fn delete_vote(&self, request_id: i64, user_id: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM mod_request_votes WHERE request_id = ?1 AND user_id = ?2",
            params![request_id, user_id],
        )
    }

    pub fn get_vote(
        &self,
        request_id: i64,
        user_id: i64,
    ) -> rusqlite::Result<Option<ModRequestVote>> {
        self.conn
            .query_row(
                "SELECT id, request_id, user_id, upvote, comment, created_at
                 FROM mod_request_votes WHERE request_id = ?1 AND user_id = ?2",
                params![request_id, user_id],
                |row| {
                    Ok(ModRequestVote {
                        id: row.get(0)?,
                        request_id: row.get(1)?,
                        user_id: row.get(2)?,
                        upvote: row.get::<_, i32>(3)? != 0,
                        comment: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                },
            )
            .optional()
    }

    pub fn list_vote_comments(&self, request_id: i64) -> rusqlite::Result<Vec<VoteComment>> {
        let mut stmt = self.conn.prepare(
            "SELECT u.username, v.upvote, v.comment, v.created_at
             FROM mod_request_votes v
             JOIN users u ON v.user_id = u.id
             WHERE v.request_id = ?1 AND v.comment IS NOT NULL AND v.comment != ''
             ORDER BY v.created_at DESC",
        )?;
        let rows = stmt.query_map(params![request_id], |row| {
            Ok(VoteComment {
                username: row.get(0)?,
                upvote: row.get::<_, i32>(1)? != 0,
                comment: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        rows.collect()
    }
}

use rusqlite::OptionalExtension;

fn row_to_request(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModRequest> {
    Ok(ModRequest {
        id: row.get(0)?,
        user_id: row.get(1)?,
        forge_mod_id: row.get(2)?,
        mod_name: row.get(3)?,
        mod_slug: row.get(4)?,
        mod_description: row.get(5)?,
        fika_compatible: row.get(6)?,
        reason: row.get(7)?,
        status: row.get(8)?,
        resolved_by: row.get(9)?,
        resolved_at: row.get(10)?,
        resolve_comment: row.get(11)?,
        created_at: row.get(12)?,
        forge_cached_at: row.get(13)?,
    })
}

fn row_to_request_view(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModRequestView> {
    let request = ModRequest {
        id: row.get(0)?,
        user_id: row.get(1)?,
        forge_mod_id: row.get(2)?,
        mod_name: row.get(3)?,
        mod_slug: row.get(4)?,
        mod_description: row.get(5)?,
        fika_compatible: row.get(6)?,
        reason: row.get(7)?,
        status: row.get(8)?,
        resolved_by: row.get(9)?,
        resolved_at: row.get(10)?,
        resolve_comment: row.get(11)?,
        created_at: row.get(12)?,
        forge_cached_at: row.get(13)?,
    };
    let upvote_count: i64 = row.get(15)?;
    let downvote_count: i64 = row.get(16)?;
    Ok(ModRequestView {
        request,
        requester_username: row.get(14)?,
        vote_score: upvote_count - downvote_count,
        upvote_count,
        downvote_count,
        comment_count: row.get(17)?,
        current_user_vote: row.get::<_, Option<i32>>(18)?.map(|v| v != 0),
        resolver_username: row.get(19)?,
    })
}
