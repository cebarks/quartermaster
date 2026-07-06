use rusqlite::{params, OptionalExtension};

use super::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    Approved,
    Queued,
    Installed,
    Rejected,
}

impl RequestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Queued => "queued",
            Self::Installed => "installed",
            Self::Rejected => "rejected",
        }
    }
}

impl std::fmt::Display for RequestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for RequestStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "queued" => Ok(Self::Queued),
            "installed" => Ok(Self::Installed),
            "rejected" => Ok(Self::Rejected),
            other => Err(format!("unknown request status: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model — fields read by Askama templates
pub struct RequestStatusLog {
    pub id: i64,
    pub request_id: i64,
    pub from_status: String,
    pub to_status: String,
    pub changed_by: Option<i64>,
    pub changed_by_username: Option<String>,
    pub changed_at: String,
    pub comment: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // SQL row model
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
#[allow(dead_code)] // SQL row model
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

    pub fn has_active_request_for_mod(&self, forge_mod_id: i64) -> rusqlite::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM mod_requests WHERE forge_mod_id = ?1
             AND status IN ('pending', 'approved', 'queued', 'installed')",
            params![forge_mod_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn transition_request_status(
        &self,
        request_id: i64,
        expected_from: &[RequestStatus],
        new_status: RequestStatus,
        changed_by: Option<i64>,
        comment: Option<&str>,
    ) -> rusqlite::Result<bool> {
        let from_strs: Vec<&str> = expected_from.iter().map(|s| s.as_str()).collect();
        let placeholders: String = (0..from_strs.len())
            .map(|i| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");

        let tx = self.conn.unchecked_transaction()?;

        // Read old status for audit log (inside transaction for consistency)
        let old_status: Option<String> = tx
            .query_row(
                &format!(
                    "SELECT status FROM mod_requests WHERE id = ?1 AND status IN ({placeholders})"
                ),
                rusqlite::params_from_iter(
                    std::iter::once(request_id.to_string())
                        .chain(from_strs.iter().map(|s| s.to_string())),
                ),
                |row| row.get(0),
            )
            .optional()?;

        let Some(old_status) = old_status else {
            tx.commit()?;
            return Ok(false);
        };

        tx.execute(
            "UPDATE mod_requests SET status = ?1 WHERE id = ?2",
            params![new_status.as_str(), request_id],
        )?;
        tx.execute(
            "INSERT INTO request_status_log (request_id, from_status, to_status, changed_by, comment)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![request_id, old_status, new_status.as_str(), changed_by, comment],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn transition_request_by_forge_mod_id(
        &self,
        forge_mod_id: i64,
        from_statuses: &[RequestStatus],
        new_status: RequestStatus,
        changed_by: Option<i64>,
        comment: Option<&str>,
    ) -> rusqlite::Result<usize> {
        let from_strs: Vec<&str> = from_statuses.iter().map(|s| s.as_str()).collect();

        let ids: Vec<(i64, String)> = {
            let mut stmt = self
                .conn
                .prepare("SELECT id, status FROM mod_requests WHERE forge_mod_id = ?1")?;
            let rows = stmt.query_map(params![forge_mod_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let matching: Vec<_> = ids
            .iter()
            .filter(|(_, status)| from_strs.contains(&status.as_str()))
            .collect();

        if matching.is_empty() {
            return Ok(0);
        }

        let tx = self.conn.unchecked_transaction()?;
        for (id, current_status) in &matching {
            tx.execute(
                "UPDATE mod_requests SET status = ?1 WHERE id = ?2",
                params![new_status.as_str(), id],
            )?;
            tx.execute(
                "INSERT INTO request_status_log (request_id, from_status, to_status, changed_by, comment)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, current_status, new_status.as_str(), changed_by, comment],
            )?;
        }
        tx.commit()?;
        Ok(matching.len())
    }

    pub fn get_request_status_log(
        &self,
        request_id: i64,
    ) -> rusqlite::Result<Vec<RequestStatusLog>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.id, l.request_id, l.from_status, l.to_status, l.changed_by,
                    u.username, l.changed_at, l.comment
             FROM request_status_log l
             LEFT JOIN users u ON l.changed_by = u.id
             WHERE l.request_id = ?1
             ORDER BY l.changed_at ASC, l.id ASC",
        )?;
        let rows = stmt.query_map(params![request_id], |row| {
            Ok(RequestStatusLog {
                id: row.get(0)?,
                request_id: row.get(1)?,
                from_status: row.get(2)?,
                to_status: row.get(3)?,
                changed_by: row.get(4)?,
                changed_by_username: row.get(5)?,
                changed_at: row.get(6)?,
                comment: row.get(7)?,
            })
        })?;
        rows.collect()
    }

    pub fn set_request_resolver(
        &self,
        request_id: i64,
        resolved_by: i64,
        comment: Option<&str>,
    ) -> rusqlite::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE mod_requests SET resolved_by = ?1, resolved_at = ?2, resolve_comment = ?3
             WHERE id = ?4 AND resolved_by IS NULL",
            params![resolved_by, now, comment, request_id],
        )?;
        Ok(())
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

    pub fn list_approved_request_ids(&self) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM mod_requests WHERE status = 'approved'")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
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
