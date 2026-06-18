use actix_session::Session;
use actix_web::web::{self, Data, Html};
use askama::Template;

use crate::health::{self, HealthReport};
use crate::web::auth::{require_auth, SessionUser};
use crate::web::error::WebError;
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "status.html")]
struct StatusPageTemplate {
    user: SessionUser,
}

#[derive(Template)]
#[template(path = "partials/status_detail.html")]
struct StatusDetailTemplate {
    report: HealthReport,
}

pub async fn status_page(session: Session) -> actix_web::Result<Html> {
    let user = require_auth(&session)?;
    let tmpl = StatusPageTemplate { user };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

pub async fn status_partial(state: Data<AppState>) -> actix_web::Result<Html> {
    let report = build_health_report(&state).await.map_err(WebError::from)?;
    let tmpl = StatusDetailTemplate { report };
    Ok(Html::new(tmpl.render().map_err(WebError::from)?))
}

async fn build_health_report(state: &AppState) -> anyhow::Result<HealthReport> {
    use crate::server_detect::resolve_server_addr;
    use crate::spt::mods::{compute_file_hash, scan_mod_directories};
    use crate::spt::server::SptClient;

    let (host, port) = resolve_server_addr(&state.config, &state.spt_dir);
    let spt_client = SptClient::new(&host, port)?;
    let address = spt_client.base_url().to_string();

    // Server check (async, no DB needed)
    let server = {
        let ping = spt_client.ping().await;
        let (reachable, latency_ms, error) = match &ping {
            Ok(p) if p.ok => (true, Some(p.latency_ms), None),
            Ok(_) => (false, None, Some("server returned error".to_string())),
            Err(e) => (false, None, Some(format!("{e:#}"))),
        };

        let (version, version_matches) = if reachable {
            let v = spt_client.server_version().await.ok();
            let matches = v.as_deref().map(|v| v == state.spt_info.spt_version);
            (v, matches)
        } else {
            (None, None)
        };

        health::ServerHealth {
            reachable,
            latency_ms,
            version,
            version_matches,
            address,
            error,
        }
    };

    // Mods check (needs DB for list, then async Forge call)
    let db = state.db.clone();
    let installed_mods = web::block(move || {
        let db = db.lock();
        db.list_mods()
    })
    .await??;

    let mods = {
        let mut updates_available = 0;
        let mut incompatible_mods = Vec::new();

        if !installed_mods.is_empty() {
            let check_list: Vec<(i64, String)> = installed_mods
                .iter()
                .map(|m| (m.forge_mod_id, m.version.clone()))
                .collect();

            if let Ok(results) = state
                .forge
                .check_updates(&check_list, &state.spt_info.spt_version)
                .await
            {
                updates_available = results.updates.len();
                for m in &results.incompatible_with_spt {
                    incompatible_mods.push(m.name.clone());
                }
            }
        }

        health::ModsHealth {
            installed_count: installed_mods.len(),
            updates_available,
            incompatible_mods,
        }
    };

    // Integrity check (sync, needs DB)
    let db = state.db.clone();
    let spt_dir = state.spt_dir.clone();
    let integrity = web::block(move || {
        let db = db.lock();
        let tracked_files = db.get_all_tracked_files()?;
        let mut missing_files = Vec::new();
        let mut modified_files = Vec::new();

        for file in &tracked_files {
            let full_path = spt_dir.join(&file.file_path);
            if !full_path.exists() {
                missing_files.push(file.file_path.clone());
                continue;
            }
            if let Some(ref expected_hash) = file.file_hash {
                match compute_file_hash(&full_path) {
                    Ok(actual_hash) if actual_hash != *expected_hash => {
                        modified_files.push(file.file_path.clone());
                    }
                    Err(_) => {
                        modified_files.push(file.file_path.clone());
                    }
                    _ => {}
                }
            }
        }

        let all_disk_files = scan_mod_directories(&spt_dir)?;
        let tracked_paths: std::collections::HashSet<&str> =
            tracked_files.iter().map(|f| f.file_path.as_str()).collect();
        let untracked: Vec<&str> = all_disk_files
            .iter()
            .filter(|f| !tracked_paths.contains(f.as_str()))
            .map(|f| f.as_str())
            .collect();

        let mut dir_counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for path in &untracked {
            let parts: Vec<&str> = path.split('/').collect();
            let dir = if path.starts_with("SPT/") && parts.len() >= 4 {
                format!("{}/{}/{}/{}", parts[0], parts[1], parts[2], parts[3])
            } else if path.starts_with("BepInEx/") && parts.len() >= 3 {
                format!("{}/{}/{}", parts[0], parts[1], parts[2])
            } else {
                path.to_string()
            };
            *dir_counts.entry(dir).or_default() += 1;
        }

        let untracked_dirs = dir_counts
            .into_iter()
            .map(|(path, file_count)| health::UntrackedDir { path, file_count })
            .collect();

        Ok::<_, anyhow::Error>(health::IntegrityHealth {
            tracked_files: tracked_files.len(),
            missing_files,
            modified_files,
            untracked_dirs,
        })
    })
    .await??;

    Ok(HealthReport {
        server,
        mods,
        integrity,
    })
}
