//! Background mod-update poller with Discord notifications.
//!
//! Polls Forge and GitHub on a timer (default 30 min) so admins hear about
//! updates without watching the dashboard, and warms the same cache the page
//! reads so it is instant afterwards.
//!
//! Only runs when a webhook is configured — no webhook, no poller.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use parking_lot::Mutex;

use crate::db::Database;
use crate::forge::client::ForgeClient;
use crate::web::update_cache::UpdateCache;

struct Available {
    mod_db_id: i64,
    name: String,
    current: String,
    new: String,
    source: &'static str,
}

/// Start the poller. Returns immediately; does nothing if no webhook is set.
pub fn spawn(
    db: Arc<Mutex<Database>>,
    forge: ForgeClient,
    update_cache: UpdateCache,
    spt_version: String,
    webhook_url: Option<String>,
    interval_secs: u64,
) {
    let Some(webhook_url) = webhook_url.filter(|u| !u.trim().is_empty()) else {
        tracing::debug!("no Discord webhook configured — update poller not started");
        return;
    };
    // A tight loop would hammer Forge and burn GitHub's 60/hr anonymous budget.
    let interval_secs = interval_secs.max(300);

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
        loop {
            ticker.tick().await;
            if let Err(e) = poll_once(&db, &forge, &update_cache, &spt_version, &webhook_url).await
            {
                tracing::warn!(err = %e, "mod update poll failed");
            }
        }
    });
    tracing::info!(
        interval_secs,
        "mod update poller started — new updates will be announced on Discord"
    );
}

async fn poll_once(
    db: &Arc<Mutex<Database>>,
    forge: &ForgeClient,
    update_cache: &UpdateCache,
    spt_version: &str,
    webhook_url: &str,
) -> Result<()> {
    let installed = { db.lock().list_mods()? };
    if installed.is_empty() {
        return Ok(());
    }

    let mut available: Vec<Available> = Vec::new();

    // Forge-sourced mods: one batched call, which also refreshes the page cache.
    let forge_list: Vec<(i64, String)> = installed
        .iter()
        .filter_map(|m| m.forge_mod_id.map(|id| (id, m.version.clone())))
        .collect();
    if !forge_list.is_empty() {
        match forge.check_updates(&forge_list, spt_version).await {
            Ok(data) => {
                for m in &installed {
                    let Some(u) = data.updates.iter().find(|u| {
                        m.forge_mod_id == Some(u.current_version.mod_id)
                            && u.recommended_version.version != m.version
                    }) else {
                        continue;
                    };
                    available.push(Available {
                        mod_db_id: m.id,
                        name: m.name.clone(),
                        current: m.version.clone(),
                        new: u.recommended_version.version.clone(),
                        source: "Forge",
                    });
                }
                update_cache.set(data);
            }
            Err(e) => tracing::warn!(err = %e, "Forge update check failed during poll"),
        }
    }

    // GitHub-sourced mods: one call per repo, memoised.
    for m in &installed {
        let Some(r) = m
            .source_url
            .as_deref()
            .and_then(crate::github::parse_release_url)
        else {
            continue;
        };
        let Some(rel) = crate::github::latest_release_cached(&r).await else {
            continue;
        };
        if rel.version != crate::github::normalize_version(&m.version) {
            available.push(Available {
                mod_db_id: m.id,
                name: m.name.clone(),
                current: m.version.clone(),
                new: rel.version,
                source: "GitHub",
            });
        }
    }

    // Announce only what has not been announced before.
    let fresh: Vec<Available> = {
        let db = db.lock();
        available
            .into_iter()
            .filter(|a| match db.was_update_notified(a.mod_db_id, &a.new) {
                Ok(seen) => !seen,
                Err(e) => {
                    tracing::warn!(err = %e, "failed to read notification state");
                    false
                }
            })
            .collect()
    };
    if fresh.is_empty() {
        return Ok(());
    }

    post_to_discord(webhook_url, &fresh).await?;

    let db = db.lock();
    for a in &fresh {
        if let Err(e) = db.mark_update_notified(a.mod_db_id, &a.new) {
            tracing::warn!(mod_name = a.name, err = %e, "failed to record notification");
        }
    }
    tracing::info!(count = fresh.len(), "announced mod updates on Discord");
    Ok(())
}

const EMBED_COLOR: u32 = 0xC7_7B_2A;

fn embed(u: &Available) -> serde_json::Value {
    serde_json::json!({
        "embeds": [{
            "title": u.name,
            "description": format!("**{}** → **{}**", u.current, u.new),
            "color": EMBED_COLOR,
            "footer": { "text": format!("Quartermaster · {}", u.source) },
        }]
    })
}

async fn post_to_discord(webhook_url: &str, updates: &[Available]) -> Result<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .build()?;

    for u in updates {
        client
            .post(webhook_url)
            .json(&embed(u))
            .send()
            .await?
            .error_for_status()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_names_both_versions_and_the_source() {
        let e = embed(&Available {
            mod_db_id: 2,
            name: "Corter-ModSync".to_string(),
            current: "0.12.5".to_string(),
            new: "0.13.0".to_string(),
            source: "GitHub",
        });
        let embeds = e["embeds"].as_array().expect("embeds array");
        assert_eq!(embeds.len(), 1);
        assert_eq!(embeds[0]["title"], "Corter-ModSync");
        assert_eq!(embeds[0]["description"], "**0.12.5** → **0.13.0**");
        assert_eq!(embeds[0]["footer"]["text"], "Quartermaster · GitHub");
    }
}
