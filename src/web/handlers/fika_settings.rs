use actix_session::Session;
use actix_web::web::{Data, Form};
use actix_web::{HttpRequest, HttpResponse};
use askama::Template;
use jsonc_parser::cst::CstInputValue;

use crate::db::rbac::Permission;
use crate::fika::config::{fika_config_path, read_fika_config, read_fika_cst, write_fika_cst};
use crate::web::auth::{require_auth, require_permission, SessionUser};
use crate::web::error::WebError;
use crate::web::flash::{set_flash, take_flash, FlashMessage, FlashType};
use crate::web::nav::NavContext;
use crate::web::state::AppState;

#[derive(Template)]
#[template(path = "fika_settings.html")]
struct FikaSettingsTemplate {
    user: SessionUser,
    flash: Option<FlashMessage>,
    csrf_token: String,
    nav: NavContext,
    config: crate::fika::config::FikaConfig,
}

pub async fn fika_settings_page(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;

    if !state.fika_installed {
        return Ok(HttpResponse::NotFound().body("Fika is not installed"));
    }

    let flash = take_flash(&session);
    let csrf_token = crate::web::csrf::get_or_create_token(&session);

    let spt_dir = state.spt_dir.clone();
    let config = actix_web::web::block(move || {
        let path = fika_config_path(&spt_dir);
        read_fika_config(&path)
    })
    .await
    .map_err(|e| WebError::Internal(anyhow::anyhow!("blocking error: {e}")))?
    .map_err(WebError::from)?;

    let tmpl = FikaSettingsTemplate {
        user,
        flash,
        csrf_token,
        nav: NavContext::from_state(&state),
        config,
    };

    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(tmpl.render().map_err(WebError::from)?))
}

#[derive(serde::Deserialize)]
pub struct FikaSettingsForm {
    csrf_token: String,
    // Client settings
    use_btr: Option<String>,
    friendly_fire: Option<String>,
    dynamic_v_exfils: Option<String>,
    allow_free_cam: Option<String>,
    allow_spectate_free_cam: Option<String>,
    blacklisted_items: String,
    force_save_on_death: Option<String>,
    mods_required: String,
    mods_optional: String,
    use_inertia: Option<String>,
    shared_quest_progression: Option<String>,
    can_edit_raid_settings: Option<String>,
    enable_transits: Option<String>,
    anyone_can_start_raid: Option<String>,
    allow_name_plates: Option<String>,
    random_labyrinth_spawns: Option<String>,
    pmc_found_in_raid: Option<String>,
    allow_spectate_bots: Option<String>,
    instant_load: Option<String>,
    fast_load: Option<String>,
    // Revive config
    revive_enabled: Option<String>,
    revive_headshot_kills: Option<String>,
    revive_grenades_kills: Option<String>,
    revive_allow_looting: Option<String>,
    revive_max_revives: u32,
    revive_bleedout_time: f64,
    revive_revive_time: f64,
    // Server settings
    spt_http_ip: String,
    spt_http_port: u16,
    spt_http_backend_ip: String,
    spt_http_backend_port: u16,
    spt_disable_chat_bots: Option<String>,
    webhook_enabled: Option<String>,
    webhook_name: String,
    webhook_avatar_url: String,
    webhook_url: String,
    allow_item_sending: Option<String>,
    item_sending_storage_time: u32,
    sent_items_lose_fir: Option<String>,
    launcher_list_all_profiles: Option<String>,
    session_timeout: u32,
    show_dev_profile: Option<String>,
    show_non_standard_profile: Option<String>,
    admin_ids: String,
    // NAT Punch
    nat_punch_enable: Option<String>,
    nat_punch_port: u16,
    // Headless
    headless_profiles_amount: u32,
    headless_scripts_generate: Option<String>,
    headless_scripts_force_ip: String,
    headless_set_level_to_average: Option<String>,
    headless_restart_after_raids: u32,
    // Background
    background_enable: Option<String>,
}

pub async fn fika_settings_save(
    state: Data<AppState>,
    req: HttpRequest,
    session: Session,
    form: Form<FikaSettingsForm>,
) -> actix_web::Result<HttpResponse> {
    let user = require_auth(&req)?;
    require_permission(&user, Permission::SettingsManage)?;

    if !state.fika_installed {
        return Ok(HttpResponse::NotFound().body("Fika is not installed"));
    }

    if !crate::web::csrf::validate_token(&session, &form.csrf_token) {
        return Err(WebError::Forbidden.into());
    }

    let spt_dir = state.spt_dir.clone();
    let form_data = form.into_inner();

    let result = actix_web::web::block(move || {
        let _guard = state.fika_config_lock.lock();
        let path = fika_config_path(&spt_dir);

        // Read CST (ALL CST ops must happen in this sync block)
        let cst = read_fika_cst(&path)?;
        let root = cst.object_value_or_set();

        // Helper to set bool
        let set_bool = |parent: &jsonc_parser::cst::CstObject, key: &str, val: bool| {
            if let Some(prop) = parent.get(key) {
                prop.set_value(CstInputValue::Bool(val));
            }
        };

        // Helper to set number
        let set_num = |parent: &jsonc_parser::cst::CstObject, key: &str, val: String| {
            if let Some(prop) = parent.get(key) {
                prop.set_value(CstInputValue::Number(val));
            }
        };

        // Helper to set string
        let set_str = |parent: &jsonc_parser::cst::CstObject, key: &str, val: &str| {
            if let Some(prop) = parent.get(key) {
                prop.set_value(CstInputValue::String(val.to_string()));
            }
        };

        // Helper to set array from comma-separated or newline-separated string
        let set_array = |parent: &jsonc_parser::cst::CstObject, key: &str, val: &str| {
            if let Some(prop) = parent.get(key) {
                let items: Vec<CstInputValue> = val
                    .lines()
                    .flat_map(|line| line.split(','))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .map(CstInputValue::String)
                    .collect();
                prop.set_value(CstInputValue::Array(items));
            }
        };

        // Client section
        if let Some(client) = root.object_value("client") {
            set_bool(&client, "useBtr", form_data.use_btr.is_some());
            set_bool(&client, "friendlyFire", form_data.friendly_fire.is_some());
            set_bool(
                &client,
                "dynamicVExfils",
                form_data.dynamic_v_exfils.is_some(),
            );
            set_bool(&client, "allowFreeCam", form_data.allow_free_cam.is_some());
            set_bool(
                &client,
                "allowSpectateFreeCam",
                form_data.allow_spectate_free_cam.is_some(),
            );
            set_array(&client, "blacklistedItems", &form_data.blacklisted_items);
            set_bool(
                &client,
                "forceSaveOnDeath",
                form_data.force_save_on_death.is_some(),
            );

            // Mods object
            if let Some(mods) = client.object_value("mods") {
                set_array(&mods, "required", &form_data.mods_required);
                set_array(&mods, "optional", &form_data.mods_optional);
            }

            set_bool(&client, "useInertia", form_data.use_inertia.is_some());
            set_bool(
                &client,
                "sharedQuestProgression",
                form_data.shared_quest_progression.is_some(),
            );
            set_bool(
                &client,
                "canEditRaidSettings",
                form_data.can_edit_raid_settings.is_some(),
            );
            set_bool(
                &client,
                "enableTransits",
                form_data.enable_transits.is_some(),
            );
            set_bool(
                &client,
                "anyoneCanStartRaid",
                form_data.anyone_can_start_raid.is_some(),
            );
            set_bool(
                &client,
                "allowNamePlates",
                form_data.allow_name_plates.is_some(),
            );
            set_bool(
                &client,
                "randomLabyrinthSpawns",
                form_data.random_labyrinth_spawns.is_some(),
            );
            set_bool(
                &client,
                "pmcFoundInRaid",
                form_data.pmc_found_in_raid.is_some(),
            );
            set_bool(
                &client,
                "allowSpectateBots",
                form_data.allow_spectate_bots.is_some(),
            );
            set_bool(&client, "instantLoad", form_data.instant_load.is_some());
            set_bool(&client, "fastLoad", form_data.fast_load.is_some());

            // Revive config
            if let Some(revive) = client.object_value("reviveConfig") {
                set_bool(&revive, "enabled", form_data.revive_enabled.is_some());
                set_bool(
                    &revive,
                    "headshotKills",
                    form_data.revive_headshot_kills.is_some(),
                );
                set_bool(
                    &revive,
                    "grenadesKills",
                    form_data.revive_grenades_kills.is_some(),
                );
                set_bool(
                    &revive,
                    "allowLooting",
                    form_data.revive_allow_looting.is_some(),
                );
                set_num(
                    &revive,
                    "maxRevives",
                    form_data.revive_max_revives.to_string(),
                );
                set_num(
                    &revive,
                    "bleedoutTime",
                    form_data.revive_bleedout_time.to_string(),
                );
                set_num(
                    &revive,
                    "reviveTime",
                    form_data.revive_revive_time.to_string(),
                );
            }
        }

        // Server section
        if let Some(server) = root.object_value("server") {
            if let Some(spt) = server.object_value("SPT") {
                if let Some(http) = spt.object_value("http") {
                    set_str(&http, "ip", &form_data.spt_http_ip);
                    set_num(&http, "port", form_data.spt_http_port.to_string());
                    set_str(&http, "backendIp", &form_data.spt_http_backend_ip);
                    set_num(
                        &http,
                        "backendPort",
                        form_data.spt_http_backend_port.to_string(),
                    );
                }
                set_bool(
                    &spt,
                    "disableSPTChatBots",
                    form_data.spt_disable_chat_bots.is_some(),
                );
            }

            if let Some(webhook) = server.object_value("webhook") {
                set_bool(&webhook, "enabled", form_data.webhook_enabled.is_some());
                set_str(&webhook, "name", &form_data.webhook_name);
                set_str(&webhook, "avatarUrl", &form_data.webhook_avatar_url);
                set_str(&webhook, "url", &form_data.webhook_url);
            }

            set_bool(
                &server,
                "allowItemSending",
                form_data.allow_item_sending.is_some(),
            );
            set_num(
                &server,
                "itemSendingStorageTime",
                form_data.item_sending_storage_time.to_string(),
            );
            set_bool(
                &server,
                "sentItemsLoseFir",
                form_data.sent_items_lose_fir.is_some(),
            );
            set_bool(
                &server,
                "launcherListAllProfiles",
                form_data.launcher_list_all_profiles.is_some(),
            );
            set_num(
                &server,
                "sessionTimeout",
                form_data.session_timeout.to_string(),
            );
            set_bool(
                &server,
                "showDevProfile",
                form_data.show_dev_profile.is_some(),
            );
            set_bool(
                &server,
                "showNonStandardProfile",
                form_data.show_non_standard_profile.is_some(),
            );
            set_array(&server, "adminIds", &form_data.admin_ids);
            // apiKey is read-only, don't update it
        }

        // NAT Punch Server
        if let Some(nat_punch) = root.object_value("natPunchServer") {
            set_bool(&nat_punch, "enable", form_data.nat_punch_enable.is_some());
            set_num(&nat_punch, "port", form_data.nat_punch_port.to_string());
        }

        // Headless
        if let Some(headless) = root.object_value("headless") {
            if let Some(profiles) = headless.object_value("profiles") {
                set_num(
                    &profiles,
                    "amount",
                    form_data.headless_profiles_amount.to_string(),
                );
                // aliases is a map, skip for now
            }
            if let Some(scripts) = headless.object_value("scripts") {
                set_bool(
                    &scripts,
                    "generate",
                    form_data.headless_scripts_generate.is_some(),
                );
                set_str(&scripts, "forceIp", &form_data.headless_scripts_force_ip);
            }
            set_bool(
                &headless,
                "setLevelToAverageOfLobby",
                form_data.headless_set_level_to_average.is_some(),
            );
            set_num(
                &headless,
                "restartAfterAmountOfRaids",
                form_data.headless_restart_after_raids.to_string(),
            );
        }

        // Background
        if let Some(background) = root.object_value("background") {
            set_bool(&background, "enable", form_data.background_enable.is_some());
        }

        // Write CST back to disk
        write_fika_cst(&cst, &path)?;

        Ok::<_, anyhow::Error>(())
    })
    .await;

    match result {
        Ok(Ok(())) => {
            set_flash(
                &session,
                "Fika settings saved. Restart the SPT server for changes to take effect.",
                FlashType::Success,
            );
        }
        Ok(Err(e)) => {
            tracing::error!(err = %e, "failed to save fika settings");
            set_flash(&session, "Failed to save Fika settings", FlashType::Error);
        }
        Err(e) => {
            tracing::error!(err = %e, "task failed saving fika settings");
            set_flash(&session, "Failed to save Fika settings", FlashType::Error);
        }
    }

    Ok(HttpResponse::SeeOther()
        .insert_header(("Location", "/quma/settings/fika"))
        .finish())
}
