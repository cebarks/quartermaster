#![allow(dead_code)]

use serde::{Deserialize, Serialize};

// =============================================================================
// Top-level SvmConfig
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct SvmConfig {
    pub preset_notes: String,
    pub items: Items,
    pub hideout: Hideout,
    pub traders: Traders,
    pub loot: Loot,
    pub player: Player,
    pub raids: Raids,
    pub fleamarket: Fleamarket,
    pub services: Services,
    pub quests: Quests,
    #[serde(rename = "CSM")]
    pub csm: Csm,
    pub scav: Scav,
    pub bots: Bots,
    #[serde(rename = "PMC")]
    pub pmc: Pmc,
    pub custom: Custom,
}

// =============================================================================
// Items Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Items {
    pub examine_keys: bool,
    pub weapon_heat_off: bool,
    #[serde(rename = "SMGToHolster")]
    pub smg_to_holster: bool,
    pub pistol_to_main: bool,
    pub all_examined_items: bool,
    pub equip_rigs_with_armors: bool,
    pub remove_secure_container_filters: bool,
    pub backpack_stacking: i32,
    pub misfire_chance: f64,
    pub fragment_mult: f64,
    pub heat_factor: f64,
    pub examine_time: f64,
    pub malfunct_chance_mult: f64,
    pub weight_changer: f64,
    pub item_price_mult: f64,
    pub enable_currency: bool,
    pub rub_stack: i32,
    pub dollar_stack: i32,
    #[serde(rename = "GPStack")]
    pub gp_stack: i32,
    pub euro_stack: i32,
    pub ammo_load_speed: f64,
    pub ammo_un_load_speed: f64,
    pub loot_exp: f64,
    pub enable_items: bool,
    pub examine_exp: f64,
    pub ammo_stacks: AmmoStacks,
    pub keys: Keys,
    pub ammo_switch: bool,
    pub remove_raid_restr: bool,
    pub remove_backpacks_restrictions: bool,
    #[serde(rename = "SurvCMSToSpec")]
    pub surv_cms_to_spec: bool,
    #[serde(rename = "SurvCMSSecConBlock")]
    pub surv_cms_sec_con_block: bool,
    pub no_gear_penalty: bool,
    pub raid_drop: bool,
}

impl Default for Items {
    fn default() -> Self {
        Self {
            examine_keys: false,
            weapon_heat_off: false,
            smg_to_holster: false,
            pistol_to_main: false,
            all_examined_items: false,
            equip_rigs_with_armors: false,
            remove_secure_container_filters: false,
            backpack_stacking: 7,
            misfire_chance: 1.0,
            fragment_mult: 1.0,
            heat_factor: 1.0,
            examine_time: 1.0,
            malfunct_chance_mult: 1.0,
            weight_changer: 1.0,
            item_price_mult: 1.0,
            enable_currency: false,
            rub_stack: 1_000_000,
            dollar_stack: 50_000,
            gp_stack: 100,
            euro_stack: 50_000,
            ammo_load_speed: 1.0,
            ammo_un_load_speed: 1.0,
            loot_exp: 1.0,
            enable_items: false,
            examine_exp: 1.0,
            ammo_stacks: AmmoStacks::default(),
            keys: Keys::default(),
            ammo_switch: false,
            remove_raid_restr: false,
            remove_backpacks_restrictions: false,
            surv_cms_to_spec: false,
            surv_cms_sec_con_block: false,
            no_gear_penalty: false,
            raid_drop: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AmmoStacks {
    pub marksman_round: i32,
    pub rifle_round: i32,
    pub shotgun_round: i32,
    pub pistol_round: i32,
    pub large_caliber_round: i32,
}

impl Default for AmmoStacks {
    fn default() -> Self {
        Self {
            marksman_round: 40,
            rifle_round: 60,
            shotgun_round: 20,
            pistol_round: 50,
            large_caliber_round: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Keys {
    pub enable_keys: bool,
    pub avoid_single_key_cards: bool,
    pub ignore_access_card: bool,
    pub avoid_single_keys: bool,
    pub avoid_marked_keys: bool,
    pub avoid_residential: bool,
    pub avoid_odd_keys: bool,
    pub key_use_mult: f64,
    pub keycard_use_mult: f64,
    pub key_durability_threshold: i32,
    pub key_card_durability_threshold: i32,
    pub infinite_keys: bool,
    pub infinite_keycards: bool,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            enable_keys: false,
            avoid_single_key_cards: false,
            ignore_access_card: false,
            avoid_single_keys: false,
            avoid_marked_keys: false,
            avoid_residential: false,
            avoid_odd_keys: false,
            key_use_mult: 1.0,
            keycard_use_mult: 1.0,
            key_durability_threshold: 40,
            key_card_durability_threshold: 10,
            infinite_keys: false,
            infinite_keycards: false,
        }
    }
}

// =============================================================================
// Hideout Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Hideout {
    pub enable_stash: bool,
    pub stash: Stash,
    pub regeneration: Regeneration,
    pub water_filter_time: i32,
    pub bitcoin_time: i32,
    pub max_bitcoins: i32,
    pub no_fuel_mult: f64,
    pub scav_case_price: f64,
    pub scav_case_time: f64,
    pub hideout_const_mult: f64,
    pub hideout_prod_mult: f64,
    pub water_filter_rate: i32,
    #[serde(rename = "GPUBoostRate")]
    pub gpu_boost_rate: f64,
    pub air_filter_rate: f64,
    pub cultist_time: f64,
    pub cultist_max_rewards: i32,
    pub fuel_consumption_rate: f64,
    pub remove_constructions_requirements: bool,
    #[serde(rename = "RemoveConstructionsFIRRequirements")]
    pub remove_constructions_fir_requirements: bool,
    pub remove_customization_requirements: bool,
    pub remove_arena_crafts: bool,
    pub remove_skill_requirements: bool,
    pub remove_trader_level_requirements: bool,
    pub enable_hideout: bool,
    pub enable_prestige: bool,
    pub prestige_collector: bool,
    pub prestige_new_beginnings: bool,
    pub prestige_areas: bool,
    pub prestige_level: i32,
    pub prestige_strength: i32,
    pub prestige_endurance: i32,
    pub prestige_charisma: i32,
    pub prestige_currency: i32,
    pub first_prestige: Prestige,
    pub second_prestige: Prestige,
    pub third_prestige: Prestige,
    pub fourth_prestige: Prestige,
}

impl Default for Hideout {
    fn default() -> Self {
        Self {
            enable_stash: false,
            stash: Stash::default(),
            regeneration: Regeneration::default(),
            water_filter_time: 325,
            bitcoin_time: 2416,
            max_bitcoins: 3,
            no_fuel_mult: 1.0,
            scav_case_price: 1.0,
            scav_case_time: 1.0,
            hideout_const_mult: 1.0,
            hideout_prod_mult: 1.0,
            water_filter_rate: 66,
            gpu_boost_rate: 1.0,
            air_filter_rate: 1.0,
            cultist_time: 1.0,
            cultist_max_rewards: 5,
            fuel_consumption_rate: 1.0,
            remove_constructions_requirements: false,
            remove_constructions_fir_requirements: false,
            remove_customization_requirements: false,
            remove_arena_crafts: false,
            remove_skill_requirements: false,
            remove_trader_level_requirements: false,
            enable_hideout: false,
            enable_prestige: false,
            prestige_collector: false,
            prestige_new_beginnings: false,
            prestige_areas: false,
            prestige_level: 55,
            prestige_strength: 20,
            prestige_endurance: 20,
            prestige_charisma: 15,
            prestige_currency: 20_000_000,
            first_prestige: Prestige {
                height: 3,
                filter: false,
                skills: 5,
                mastery: 5,
            },
            second_prestige: Prestige {
                height: 4,
                filter: false,
                skills: 10,
                mastery: 10,
            },
            third_prestige: Prestige {
                height: 5,
                filter: false,
                skills: 15,
                mastery: 15,
            },
            fourth_prestige: Prestige {
                height: 6,
                filter: false,
                skills: 20,
                mastery: 20,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Stash {
    #[serde(rename = "StashTUE")]
    pub stash_tue: i32,
    pub stash_lvl4: i32,
    pub stash_lvl3: i32,
    pub stash_lvl2: i32,
    pub stash_lvl1: i32,
}

impl Default for Stash {
    fn default() -> Self {
        Self {
            stash_tue: 72,
            stash_lvl4: 68,
            stash_lvl3: 50,
            stash_lvl2: 40,
            stash_lvl1: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Regeneration {
    pub offline_regen: bool,
    pub health_regen: f64,
    pub hideout_health: bool,
    pub hideout_energy: bool,
    pub hideout_hydration: bool,
    pub hydration_regen: f64,
    pub energy_regen: f64,
}

impl Default for Regeneration {
    fn default() -> Self {
        Self {
            offline_regen: false,
            health_regen: 1.0,
            hideout_health: false,
            hideout_energy: false,
            hideout_hydration: false,
            hydration_regen: 1.0,
            energy_regen: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct Prestige {
    pub height: i32,
    pub filter: bool,
    pub skills: i32,
    pub mastery: i32,
}

// =============================================================================
// Traders Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Traders {
    pub fence: Fence,
    pub quest_redeem_default: i32,
    pub quest_redeem_unheard: i32,
    pub trader_markup: TraderMarkup,
    pub trader_sell: TraderSell,
    pub min_durab_sell: i32,
    pub remove_time_condition: bool,
    pub all_quests_available: bool,
    pub barter_offers: f64,
    pub currency_offers: f64,
    pub barter_restrictions: f64,
    pub currency_restrictions: f64,
    pub randomize_assort: bool,
    pub unlock_quest_assort: bool,
    pub enable_traders: bool,
    #[serde(rename = "FIRRestrictsQuests")]
    pub fir_restricts_quests: bool,
    pub traders_lvl4: bool,
    #[serde(rename = "FIRTrade")]
    pub fir_trade: bool,
    pub planting_time: f64,
    pub unlock_jaeger: bool,
    pub unlock_ref: bool,
    pub light_keeper: LightKeeper,
}

impl Default for Traders {
    fn default() -> Self {
        Self {
            fence: Fence::default(),
            quest_redeem_default: 48,
            quest_redeem_unheard: 72,
            trader_markup: TraderMarkup::default(),
            trader_sell: TraderSell::default(),
            min_durab_sell: 60,
            remove_time_condition: false,
            all_quests_available: false,
            barter_offers: 1.0,
            currency_offers: 1.0,
            barter_restrictions: 1.0,
            currency_restrictions: 1.0,
            randomize_assort: false,
            unlock_quest_assort: false,
            enable_traders: false,
            fir_restricts_quests: false,
            traders_lvl4: false,
            fir_trade: false,
            planting_time: 1.0,
            unlock_jaeger: false,
            unlock_ref: false,
            light_keeper: LightKeeper::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Fence {
    pub enable_fence: bool,
    pub armor_durability_max: i32,
    pub gun_durability_max: i32,
    pub armor_durability_min: i32,
    pub gun_durability_min: i32,
    pub price_mult: f64,
    pub premium_amount_on_sale: i32,
    pub preset_count: i32,
    pub stock_time_min: i32,
    pub stock_time_max: i32,
    pub amount_on_sale: i32,
    pub preset_mult: f64,
}

impl Default for Fence {
    fn default() -> Self {
        Self {
            enable_fence: false,
            armor_durability_max: 60,
            gun_durability_max: 60,
            armor_durability_min: 35,
            gun_durability_min: 35,
            price_mult: 1.2,
            premium_amount_on_sale: 50,
            preset_count: 5,
            stock_time_min: 50,
            stock_time_max: 150,
            amount_on_sale: 140,
            preset_mult: 2.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct TraderMarkup {
    pub ragman: i32,
    pub peacekeeper: i32,
    pub fence: i32,
    pub prapor: i32,
    pub jaeger: i32,
    pub ref_field: i32,
    pub mechanic: i32,
    pub skier: i32,
    pub therapist: i32,
}

impl Default for TraderMarkup {
    fn default() -> Self {
        Self {
            ragman: 62,
            peacekeeper: 45,
            fence: 40,
            prapor: 50,
            jaeger: 60,
            ref_field: 56,
            mechanic: 56,
            skier: 49,
            therapist: 63,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct TraderSell {
    pub ragman: f64,
    pub peacekeeper: f64,
    pub prapor: f64,
    pub jaeger: f64,
    pub mechanic: f64,
    pub skier: f64,
    pub ref_field: f64,
    pub therapist: f64,
}

impl Default for TraderSell {
    fn default() -> Self {
        Self {
            ragman: 1.0,
            peacekeeper: 1.0,
            prapor: 1.0,
            jaeger: 1.0,
            mechanic: 1.0,
            skier: 1.0,
            ref_field: 1.0,
            therapist: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct LightKeeper {
    pub access_time: i32,
    pub leave_time: i32,
    pub enable_light_keeper: bool,
}

impl Default for LightKeeper {
    fn default() -> Self {
        Self {
            access_time: 10,
            leave_time: 1,
            enable_light_keeper: false,
        }
    }
}

// =============================================================================
// Loot Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct Loot {
    pub airdrops: Airdrops,
    pub enable_loot: bool,
    pub locations: Locations,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Airdrops {
    pub mixed: AirdropContents,
    pub medical: AirdropContents,
    pub barter: AirdropContents,
    pub weapon: AirdropContents,
    pub sandbox_air: i32,
    pub streets_air: i32,
    pub airtime_min: i32,
    pub airtime_max: i32,
    pub lighthouse_air: i32,
    pub bigmap_air: i32,
    pub interchange_air: i32,
    pub shoreline_air: i32,
    pub reserve_air: i32,
    pub woods_air: i32,
}

impl Default for Airdrops {
    fn default() -> Self {
        Self {
            mixed: AirdropContents {
                armor_min: 1,
                armor_max: 5,
                barter_min: 15,
                barter_max: 35,
                preset_min: 3,
                preset_max: 5,
                crates_min: 1,
                crates_max: 2,
            },
            medical: AirdropContents {
                armor_min: 0,
                armor_max: 0,
                barter_min: 25,
                barter_max: 45,
                preset_min: 0,
                preset_max: 0,
                crates_min: 0,
                crates_max: 0,
            },
            barter: AirdropContents {
                armor_min: 0,
                armor_max: 0,
                barter_min: 20,
                barter_max: 35,
                preset_min: 0,
                preset_max: 0,
                crates_min: 0,
                crates_max: 0,
            },
            weapon: AirdropContents {
                armor_min: 3,
                armor_max: 6,
                barter_min: 11,
                barter_max: 22,
                preset_min: 6,
                preset_max: 8,
                crates_min: 0,
                crates_max: 2,
            },
            sandbox_air: 13,
            streets_air: 13,
            airtime_min: 1,
            airtime_max: 5,
            lighthouse_air: 20,
            bigmap_air: 20,
            interchange_air: 20,
            shoreline_air: 20,
            reserve_air: 10,
            woods_air: 25,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct AirdropContents {
    pub armor_min: i32,
    pub armor_max: i32,
    pub barter_min: i32,
    pub barter_max: i32,
    pub preset_min: i32,
    pub preset_max: i32,
    pub crates_min: i32,
    pub crates_max: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Locations {
    pub streets: LootOnLocations,
    pub sandbox: LootOnLocations,
    pub sandbox_hard: LootOnLocations,
    pub lighthouse: LootOnLocations,
    pub bigmap: LootOnLocations,
    pub interchange: LootOnLocations,
    pub factory_day: LootOnLocations,
    pub laboratory: LootOnLocations,
    pub shoreline: LootOnLocations,
    pub reserve: LootOnLocations,
    pub woods: LootOnLocations,
    pub labyrinth: LootOnLocations,
    pub factory_night: LootOnLocations,
    pub all_containers: bool,
}

impl Default for Locations {
    fn default() -> Self {
        Self {
            streets: LootOnLocations {
                loose: 3.0,
                container: 1.0,
            },
            sandbox: LootOnLocations {
                loose: 2.8,
                container: 1.0,
            },
            sandbox_hard: LootOnLocations {
                loose: 2.8,
                container: 1.0,
            },
            lighthouse: LootOnLocations {
                loose: 2.8,
                container: 1.0,
            },
            bigmap: LootOnLocations {
                loose: 2.5,
                container: 1.0,
            },
            interchange: LootOnLocations {
                loose: 2.8,
                container: 1.0,
            },
            factory_day: LootOnLocations {
                loose: 3.5,
                container: 1.0,
            },
            laboratory: LootOnLocations {
                loose: 2.8,
                container: 1.0,
            },
            shoreline: LootOnLocations {
                loose: 3.7,
                container: 1.0,
            },
            reserve: LootOnLocations {
                loose: 2.9,
                container: 1.0,
            },
            woods: LootOnLocations {
                loose: 1.9,
                container: 1.0,
            },
            labyrinth: LootOnLocations {
                loose: 3.0,
                container: 1.0,
            },
            factory_night: LootOnLocations {
                loose: 3.5,
                container: 1.0,
            },
            all_containers: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct LootOnLocations {
    pub loose: f64,
    pub container: f64,
}

// =============================================================================
// Player Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Player {
    pub enable_fatigue: bool,
    #[serde(rename = "PMCStats")]
    pub pmc_stats: Stats,
    pub char_xp: CharXp,
    pub raid_mult: RaidMult,
    pub enable_stats: bool,
    pub skills: Skills,
    pub fall_damage: bool,
    pub black_stomach: f64,
    pub hydration_loss: f64,
    pub energy_loss: f64,
    pub enable_health: bool,
    pub skill_prog_mult: f64,
    pub health: Health,
    pub weapon_skill_mult: f64,
    pub enable_player: bool,
    pub died_health: DiedHealth,
    pub max_stamina_legs: i32,
    pub max_stamina_hands: i32,
    pub enable_stamina_hands: bool,
    pub enable_stamina_legs: bool,
    pub regen_stamina_legs: f64,
    pub regen_stamina_hands: f64,
    pub jump_consumption: i32,
    pub lay_to_stand: i32,
    pub crouch_to_stand: i32,
    pub standing: f64,
    pub laying_down: f64,
    pub crouching: f64,
    pub unlimited_stamina: bool,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            enable_fatigue: false,
            pmc_stats: Stats::default(),
            char_xp: CharXp::default(),
            raid_mult: RaidMult::default(),
            enable_stats: false,
            skills: Skills::default(),
            fall_damage: false,
            black_stomach: 5.0,
            hydration_loss: 1.0,
            energy_loss: 1.0,
            enable_health: false,
            skill_prog_mult: 0.4,
            health: Health {
                head: 35,
                chest: 85,
                stomach: 70,
                left_arm: 60,
                left_leg: 65,
                right_arm: 60,
                right_leg: 65,
            },
            weapon_skill_mult: 1.0,
            enable_player: false,
            died_health: DiedHealth::default(),
            max_stamina_legs: 115,
            max_stamina_hands: 80,
            enable_stamina_hands: false,
            enable_stamina_legs: false,
            regen_stamina_legs: 4.5,
            regen_stamina_hands: 2.1,
            jump_consumption: 14,
            lay_to_stand: 20,
            crouch_to_stand: 11,
            standing: 1.0,
            laying_down: 0.15,
            crouching: 0.75,
            unlimited_stamina: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct Health {
    pub left_arm: i32,
    pub right_arm: i32,
    pub head: i32,
    pub chest: i32,
    pub stomach: i32,
    pub left_leg: i32,
    pub right_leg: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DiedHealth {
    pub saveeffects: bool,
    pub savehealth: bool,
    pub health_blacked: f64,
    pub health_death: f64,
}

impl Default for DiedHealth {
    fn default() -> Self {
        Self {
            saveeffects: true,
            savehealth: true,
            health_blacked: 0.1,
            health_death: 0.3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct CharXp {
    pub scav_kill: i32,
    #[serde(rename = "ScavHMult")]
    pub scav_h_mult: f64,
    #[serde(rename = "PMCKill")]
    pub pmc_kill: i32,
    #[serde(rename = "PMCHMult")]
    pub pmc_h_mult: f64,
}

impl Default for CharXp {
    fn default() -> Self {
        Self {
            scav_kill: 80,
            scav_h_mult: 1.1,
            pmc_kill: 175,
            pmc_h_mult: 1.2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RaidMult {
    #[serde(rename = "MIA")]
    pub mia: f64,
    pub runner: f64,
    pub survived: f64,
    pub killed: f64,
}

impl Default for RaidMult {
    fn default() -> Self {
        Self {
            mia: 1.0,
            runner: 0.5,
            survived: 1.3,
            killed: 1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Skills {
    pub skill_fatigue_reset: i32,
    pub skill_fresh_effect: f64,
    pub skill_f_points: i32,
    pub skill_points_before_fatigue: i32,
    pub skill_min_effect: f64,
    pub skill_fatigue_per_point: f64,
}

impl Default for Skills {
    fn default() -> Self {
        Self {
            skill_fatigue_reset: 200,
            skill_fresh_effect: 1.3,
            skill_f_points: 1,
            skill_points_before_fatigue: 1,
            skill_min_effect: 0.0001,
            skill_fatigue_per_point: 0.6,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Stats {
    pub max_hydration: i32,
    pub max_energy: i32,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            max_hydration: 100,
            max_energy: 100,
        }
    }
}

// =============================================================================
// Raids Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Raids {
    pub sandbox_access_level: i32,
    pub raid_time: i32,
    pub save_quest_items: bool,
    pub exfils: Exfils,
    pub no_run_through: bool,
    pub timeacceleration: i32,
    pub safe_exit: bool,
    pub save_gear_after_death: bool,
    pub raid_events: RaidEvents,
    pub lab_insurance: bool,
    pub enable_raids: bool,
    pub removelabkey: bool,
    pub on_survived_state: i32,
    pub on_killed_state: i32,
    pub on_left_state: i32,
    pub on_runner_state: i32,
    #[serde(rename = "OnMIAState")]
    pub on_mia_state: i32,
    pub enable_car_coop: bool,
    #[serde(rename = "ForceBTRFriendly")]
    pub force_btr_friendly: bool,
    pub force_transit_stash: bool,
    pub transit_height: i32,
    pub transit_width: i32,
    #[serde(rename = "ForceBTRStash")]
    pub force_btr_stash: bool,
    #[serde(rename = "EnableBTR")]
    pub enable_btr: bool,
    #[serde(rename = "BTRCoverPrice")]
    pub btr_cover_price: i32,
    #[serde(rename = "BTRTaxiPrice")]
    pub btr_taxi_price: i32,
    #[serde(rename = "BTRWoodsTimeMin")]
    pub btr_woods_time_min: i32,
    #[serde(rename = "BTRWoodsTimeMax")]
    pub btr_woods_time_max: i32,
    #[serde(rename = "BTRWoodsChance")]
    pub btr_woods_chance: i32,
    #[serde(rename = "BTRStreetsChance")]
    pub btr_streets_chance: i32,
    #[serde(rename = "BTRStreetsTimeMin")]
    pub btr_streets_time_min: i32,
    #[serde(rename = "BTRStreetsTimeMax")]
    pub btr_streets_time_max: i32,
    pub usec_mult: f64,
    pub bear_mult: f64,
    pub scav_mult: f64,
    #[serde(rename = "BTRHeight")]
    pub btr_height: i32,
    #[serde(rename = "BTRWidth")]
    pub btr_width: i32,
    pub season: i32,
    pub force_season: bool,
    pub raid_startup: RaidStartup,
}

impl Default for Raids {
    fn default() -> Self {
        Self {
            sandbox_access_level: 20,
            raid_time: 0,
            save_quest_items: false,
            exfils: Exfils::default(),
            no_run_through: false,
            timeacceleration: 7,
            safe_exit: false,
            save_gear_after_death: false,
            raid_events: RaidEvents::default(),
            lab_insurance: false,
            enable_raids: false,
            removelabkey: false,
            on_survived_state: 0,
            on_killed_state: 1,
            on_left_state: 2,
            on_runner_state: 3,
            on_mia_state: 4,
            enable_car_coop: false,
            force_btr_friendly: false,
            force_transit_stash: false,
            transit_height: 2,
            transit_width: 5,
            force_btr_stash: false,
            enable_btr: false,
            btr_cover_price: 30_000,
            btr_taxi_price: 7_000,
            btr_woods_time_min: 5,
            btr_woods_time_max: 10,
            btr_woods_chance: 50,
            btr_streets_chance: 50,
            btr_streets_time_min: 5,
            btr_streets_time_max: 10,
            usec_mult: 1.5,
            bear_mult: 1.0,
            scav_mult: 0.8,
            btr_height: 2,
            btr_width: 5,
            season: 0,
            force_season: false,
            raid_startup: RaidStartup::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Exfils {
    pub car_sandbox: i32,
    pub car_shoreline: i32,
    pub coop_paid_sandbox: i32,
    pub coop_paid_shoreline: i32,
    pub coop_paid_streets: i32,
    pub coop_paid_lighthouse: i32,
    pub car_lighthouse: i32,
    pub car_extract_time: i32,
    pub armor_extract: bool,
    pub coop_paid: bool,
    pub fence_gift: bool,
    pub coop_paid_interchange: i32,
    pub coop_paid_woods: i32,
    pub coop_paid_reserve: i32,
    pub disable_transits: bool,
    pub no_backpack: bool,
    pub free_coop: bool,
    pub car_interchange: i32,
    pub car_woods: i32,
    pub car_streets: i32,
    pub car_customs: i32,
    pub coop_paid_customs: i32,
    pub extended_extracts: bool,
    pub chance_extracts: bool,
    pub gear_extract: bool,
}

impl Default for Exfils {
    fn default() -> Self {
        Self {
            car_sandbox: 5000,
            car_shoreline: 5000,
            coop_paid_sandbox: 5000,
            coop_paid_shoreline: 5000,
            coop_paid_streets: 5000,
            coop_paid_lighthouse: 5000,
            car_lighthouse: 5000,
            car_extract_time: 60,
            armor_extract: false,
            coop_paid: false,
            fence_gift: false,
            coop_paid_interchange: 5000,
            coop_paid_woods: 5000,
            coop_paid_reserve: 5000,
            disable_transits: false,
            no_backpack: false,
            free_coop: false,
            car_interchange: 5000,
            car_woods: 5000,
            car_streets: 5000,
            car_customs: 5000,
            coop_paid_customs: 5000,
            extended_extracts: false,
            chance_extracts: false,
            gear_extract: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RaidEvents {
    pub disable_events: bool,
    pub killa_factory_chance: i32,
    pub cultist_bosses_chance: i32,
    pub goons_factory_chance: i32,
    pub cultist_bosses: bool,
    pub goons_factory: bool,
    pub bosses_on_customs: bool,
    pub bosses_on_health_resort: bool,
    pub tagilla_interchange: bool,
    pub health_resort_include_guards: bool,
    pub hounds_woods: i32,
    pub hounds_customs: i32,
    pub skier_fighters: i32,
    pub peace_fighters: i32,
    pub christmas: bool,
    pub non_seasonal_quests: bool,
    pub halloween: bool,
    pub disable_zombies: bool,
    #[serde(rename = "DisableHalloweenAIFriendly")]
    pub disable_halloween_ai_friendly: bool,
    pub include_street_bosses: bool,
    pub killa_factory: bool,
    pub bosses_on_reserve: bool,
    #[serde(rename = "AITypeOverride")]
    pub ai_type_override: bool,
    #[serde(rename = "AIType")]
    pub ai_type: i32,
    pub glukhar_labs: bool,
}

impl Default for RaidEvents {
    fn default() -> Self {
        Self {
            disable_events: false,
            killa_factory_chance: 100,
            cultist_bosses_chance: 100,
            goons_factory_chance: 100,
            cultist_bosses: false,
            goons_factory: false,
            bosses_on_customs: false,
            bosses_on_health_resort: false,
            tagilla_interchange: false,
            health_resort_include_guards: false,
            hounds_woods: 5,
            hounds_customs: 5,
            skier_fighters: 4,
            peace_fighters: 15,
            christmas: false,
            non_seasonal_quests: false,
            halloween: false,
            disable_zombies: false,
            disable_halloween_ai_friendly: false,
            include_street_bosses: false,
            killa_factory: false,
            bosses_on_reserve: false,
            ai_type_override: false,
            ai_type: 0,
            glukhar_labs: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RaidStartup {
    pub enable_raid_startup: bool,
    pub time_before_deploy_local: i32,
    #[serde(rename = "AIAmount")]
    pub ai_amount: i32,
    pub save_loot: bool,
    #[serde(rename = "AIDifficulty")]
    pub ai_difficulty: i32,
    #[serde(rename = "MIAEndofRaid")]
    pub mia_endof_raid: bool,
    pub tagged_and_cursed: bool,
    pub enable_bosses: bool,
    pub scav_wars: bool,
}

impl Default for RaidStartup {
    fn default() -> Self {
        Self {
            enable_raid_startup: false,
            time_before_deploy_local: 10,
            ai_amount: 0,
            save_loot: false,
            ai_difficulty: 0,
            mia_endof_raid: true,
            tagged_and_cursed: false,
            enable_bosses: true,
            scav_wars: false,
        }
    }
}

// =============================================================================
// Fleamarket Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Fleamarket {
    pub enable_flea_conditions: bool,
    pub enable_player_offers: bool,
    #[serde(rename = "FleaFIR")]
    pub flea_fir: bool,
    #[serde(rename = "FleaNoFIRSell")]
    pub flea_no_fir_sell: bool,
    pub event_offers: bool,
    pub sell_offers_amount: i32,
    pub flea_conditions: FleaConditions,
    pub override_offers: bool,
    pub flea_market_level: i32,
    #[serde(rename = "DisableBSGList")]
    pub disable_bsg_list: bool,
    pub enable_fleamarket: bool,
    pub sell_mult: f64,
    pub tradeoffer_max: i32,
    pub rep_loss: f64,
    pub tiered_flea: bool,
    pub rep_gain: f64,
    pub tradeoffer_min: i32,
    pub sell_chance: i32,
    pub fees_mult: f64,
    pub dynamic_offers: DynamicOffers,
}

impl Default for Fleamarket {
    fn default() -> Self {
        Self {
            enable_flea_conditions: false,
            enable_player_offers: false,
            flea_fir: false,
            flea_no_fir_sell: false,
            event_offers: false,
            sell_offers_amount: 10,
            flea_conditions: FleaConditions::default(),
            override_offers: false,
            flea_market_level: 15,
            disable_bsg_list: false,
            enable_fleamarket: false,
            sell_mult: 1.24,
            tradeoffer_max: 1,
            rep_loss: 0.03,
            tiered_flea: true,
            rep_gain: 0.02,
            tradeoffer_min: 0,
            sell_chance: 50,
            fees_mult: 1.0,
            dynamic_offers: DynamicOffers::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct FleaConditions {
    pub flea_food_min: i32,
    pub flea_armor_min: i32,
    pub flea_food_max: i32,
    pub flea_armor_max: i32,
    pub flea_medical_min: i32,
    pub flea_spec_min: i32,
    pub flea_medical_max: i32,
    pub flea_spec_max: i32,
    pub flea_weapons_min: i32,
    pub flea_vests_min: i32,
    pub flea_keys_min: i32,
    pub flea_weapons_max: i32,
    pub flea_vests_max: i32,
    pub flea_keys_max: i32,
}

impl Default for FleaConditions {
    fn default() -> Self {
        Self {
            flea_food_min: 5,
            flea_armor_min: 5,
            flea_food_max: 100,
            flea_armor_max: 100,
            flea_medical_min: 60,
            flea_spec_min: 2,
            flea_medical_max: 100,
            flea_spec_max: 100,
            flea_weapons_min: 60,
            flea_vests_min: 5,
            flea_keys_min: 97,
            flea_weapons_max: 100,
            flea_vests_max: 100,
            flea_keys_max: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct DynamicOffers {
    pub expire_threshold: i32,
    pub bundle_offer_chance: i32,
    pub barter_chance: i32,
    pub stack_min: i32,
    pub per_offer_min: i32,
    pub stack_max: i32,
    pub per_offer_max: i32,
    pub eurooffers: i32,
    pub dollaroffers: i32,
    pub roubleoffers: i32,
    pub non_stack_min: i32,
    pub time_min: i32,
    pub price_min: f64,
    pub non_stack_max: i32,
    pub time_max: i32,
    pub price_max: f64,
}

impl Default for DynamicOffers {
    fn default() -> Self {
        Self {
            expire_threshold: 1400,
            bundle_offer_chance: 6,
            barter_chance: 20,
            stack_min: 10,
            per_offer_min: 7,
            stack_max: 600,
            per_offer_max: 30,
            eurooffers: 8,
            dollaroffers: 14,
            roubleoffers: 78,
            non_stack_min: 1,
            time_min: 6,
            price_min: 0.8,
            non_stack_max: 10,
            time_max: 60,
            price_max: 1.2,
        }
    }
}

// =============================================================================
// Services Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Services {
    pub repair_box: RepairBox,
    pub enable_heal_markup: bool,
    pub enable_insurance: bool,
    pub enable_time_override: bool,
    pub free_heal_lvl: i32,
    pub free_heal_raids: i32,
    pub return_chance_prapor: i32,
    pub return_chance_therapist: i32,
    pub insurance_interval: i32,
    pub insurance_time_override: i32,
    pub insurance_attachment_chance: i32,
    pub therapist_storage_time: i32,
    pub prapor_storage_time: i32,
    pub prapor_max: i32,
    pub prapor_min: i32,
    pub therapist_max: i32,
    pub therapist_min: i32,
    pub therapist_lvl1: f64,
    pub therapist_lvl2: f64,
    pub therapist_lvl3: f64,
    pub therapist_lvl4: f64,
    pub insurance_mult_therapist_lvl1: f64,
    pub insurance_mult_therapist_lvl2: f64,
    pub insurance_mult_therapist_lvl3: f64,
    pub insurance_mult_therapist_lvl4: f64,
    pub insurance_mult_prapor_lvl1: f64,
    pub insurance_mult_prapor_lvl2: f64,
    pub insurance_mult_prapor_lvl3: f64,
    pub insurance_mult_prapor_lvl4: f64,
    pub enable_services: bool,
    pub enable_repair: bool,
    pub clothes_any_side: bool,
    pub clothes_level_unlock: bool,
    pub clothes_free: bool,
    pub scav_clothes: bool,
}

impl Default for Services {
    fn default() -> Self {
        Self {
            repair_box: RepairBox::default(),
            enable_heal_markup: false,
            enable_insurance: false,
            enable_time_override: false,
            free_heal_lvl: 5,
            free_heal_raids: 30,
            return_chance_prapor: 85,
            return_chance_therapist: 95,
            insurance_interval: 600,
            insurance_time_override: 30,
            insurance_attachment_chance: 10,
            therapist_storage_time: 144,
            prapor_storage_time: 96,
            prapor_max: 36,
            prapor_min: 24,
            therapist_max: 24,
            therapist_min: 12,
            therapist_lvl1: 1.0,
            therapist_lvl2: 1.1,
            therapist_lvl3: 1.2,
            therapist_lvl4: 1.35,
            insurance_mult_therapist_lvl1: 20.0,
            insurance_mult_therapist_lvl2: 21.0,
            insurance_mult_therapist_lvl3: 22.0,
            insurance_mult_therapist_lvl4: 23.0,
            insurance_mult_prapor_lvl1: 16.0,
            insurance_mult_prapor_lvl2: 17.0,
            insurance_mult_prapor_lvl3: 18.0,
            insurance_mult_prapor_lvl4: 19.0,
            enable_services: false,
            enable_repair: false,
            clothes_any_side: false,
            clothes_level_unlock: false,
            clothes_free: false,
            scav_clothes: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RepairBox {
    pub no_random_repair: bool,
    pub op_gun_repair: bool,
    pub armor_skill_mult: f64,
    pub weapon_maintenance_skill_mult: f64,
    pub intellect_skill_mult_weapon_kit: f64,
    pub intellect_skill_mult_armor_kit: f64,
    pub intellect_skill_limit_traders: f64,
    pub intellect_skill_limit_kit: f64,
    pub op_armor_repair: bool,
    pub repair_mult: f64,
}

impl Default for RepairBox {
    fn default() -> Self {
        Self {
            no_random_repair: false,
            op_gun_repair: false,
            armor_skill_mult: 0.05,
            weapon_maintenance_skill_mult: 0.6,
            intellect_skill_mult_weapon_kit: 0.045,
            intellect_skill_mult_armor_kit: 0.03,
            intellect_skill_limit_traders: 0.6,
            intellect_skill_limit_kit: 0.6,
            op_armor_repair: false,
            repair_mult: 1.0,
        }
    }
}

// =============================================================================
// Quests Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Quests {
    pub enable_quests_misc: bool,
    pub quest_cost_mult: f64,
    pub quest_rep_to_zero: bool,
    pub daily_quests: DailyQuests,
    pub weekly_quests: DailyQuests,
    pub enable_quests: bool,
    pub scav_quests: DailyQuests,
}

impl Default for Quests {
    fn default() -> Self {
        Self {
            enable_quests_misc: false,
            quest_cost_mult: 1.0,
            quest_rep_to_zero: false,
            daily_quests: DailyQuests::daily_default(),
            weekly_quests: DailyQuests::weekly_default(),
            enable_quests: false,
            scav_quests: DailyQuests::scav_default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DailyQuests {
    pub types: i32,
    pub reroll: i32,
    #[serde(rename = "LR1")]
    pub lr1: LevelRanges,
    #[serde(rename = "LR2")]
    pub lr2: LevelRanges,
    #[serde(rename = "LR3")]
    pub lr3: LevelRanges,
    pub access: i32,
    pub quest_amount: i32,
    pub lifespan: i32,
    pub levels: String,
    pub experience: String,
    pub items_reward: String,
    pub reputation: String,
    pub skill_point: String,
    pub skill_chance: String,
    pub roubles: String,
    #[serde(rename = "GPcoins")]
    pub gp_coins: String,
}

impl DailyQuests {
    pub fn daily_default() -> Self {
        Self {
            types: 6,
            reroll: 2,
            lr1: LevelRanges {
                min_kills: 2,
                max_kills: 4,
                min_items: 1,
                max_items: 4,
                min_extracts: 1,
                max_extracts: 3,
                min_spec_exits: 1,
                max_spec_exits: 2,
            },
            lr2: LevelRanges {
                min_kills: 5,
                max_kills: 15,
                min_items: 2,
                max_items: 4,
                min_extracts: 2,
                max_extracts: 7,
                min_spec_exits: 1,
                max_spec_exits: 3,
            },
            lr3: LevelRanges {
                min_kills: 5,
                max_kills: 20,
                min_items: 3,
                max_items: 6,
                min_extracts: 3,
                max_extracts: 15,
                min_spec_exits: 2,
                max_spec_exits: 4,
            },
            access: 5,
            quest_amount: 3,
            lifespan: 1440,
            levels: "1,10,20,30,40,50,60".to_string(),
            experience: "1000,2000,8000,13000,19000,24000,30000".to_string(),
            reputation: "0.01,0.02,0.03,0.03,0.03,0.03,0.03".to_string(),
            items_reward: "2,3,4,5,5,5,5".to_string(),
            roubles: "11000,20000,32000,45000,58000,70000,82000".to_string(),
            gp_coins: "3,3,6,6,8,8,10".to_string(),
            skill_chance: "0,1,5,10,10,15,15".to_string(),
            skill_point: "10,15,20,25,30,35,40".to_string(),
        }
    }

    pub fn weekly_default() -> Self {
        Self {
            types: 6,
            reroll: 0,
            lr1: LevelRanges {
                min_kills: 8,
                max_kills: 20,
                min_items: 4,
                max_items: 6,
                min_extracts: 3,
                max_extracts: 5,
                min_spec_exits: 1,
                max_spec_exits: 4,
            },
            lr2: LevelRanges {
                min_kills: 15,
                max_kills: 40,
                min_items: 4,
                max_items: 8,
                min_extracts: 4,
                max_extracts: 8,
                min_spec_exits: 2,
                max_spec_exits: 5,
            },
            lr3: LevelRanges {
                min_kills: 20,
                max_kills: 40,
                min_items: 6,
                max_items: 12,
                min_extracts: 5,
                max_extracts: 15,
                min_spec_exits: 3,
                max_spec_exits: 6,
            },
            access: 15,
            quest_amount: 1,
            lifespan: 10080,
            levels: "1,10,20,30,40,50,60".to_string(),
            experience: "7500,18000,30000,80000,210000,260000,350000".to_string(),
            reputation: "0.02,0.03,0.04,0.04,0.05,0.06,0.07".to_string(),
            items_reward: "3,4,5,5,5,5,5".to_string(),
            roubles: "20000,50000,175000,350000,540000,710000,880000".to_string(),
            gp_coins: "10,10,16,16,20,30,35".to_string(),
            skill_chance: "0,5,10,15,20,20,20".to_string(),
            skill_point: "25,35,45,50,55,60,65".to_string(),
        }
    }

    pub fn scav_default() -> Self {
        Self {
            types: 6,
            reroll: 2,
            lr1: LevelRanges {
                min_kills: 1,
                max_kills: 3,
                min_items: 2,
                max_items: 5,
                min_extracts: 1,
                max_extracts: 3,
                min_spec_exits: 1,
                max_spec_exits: 3,
            },
            lr2: LevelRanges {
                min_kills: 3,
                max_kills: 9,
                min_items: 2,
                max_items: 5,
                min_extracts: 2,
                max_extracts: 7,
                min_spec_exits: 1,
                max_spec_exits: 3,
            },
            lr3: LevelRanges {
                min_kills: 0,
                max_kills: 0,
                min_items: 4,
                max_items: 6,
                min_extracts: 3,
                max_extracts: 15,
                min_spec_exits: 2,
                max_spec_exits: 4,
            },
            access: 1,
            quest_amount: 1,
            lifespan: 1440,
            levels: "1,10,20,30,40,50,60".to_string(),
            experience: "0,0,0,0,0,0,0".to_string(),
            reputation: "0.02,0.02,0.03,0.03,0.04,0.04,0.05".to_string(),
            items_reward: "2,3,3,3,3,4,4".to_string(),
            roubles: "11000,20000,32000,45000,58000,70000,82000".to_string(),
            gp_coins: "1,1,2,2,4,4,5".to_string(),
            skill_chance: "0,0,0,0,0,0,0".to_string(),
            skill_point: "10,15,20,25,30,35,40".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct LevelRanges {
    pub min_kills: i32,
    pub max_kills: i32,
    pub min_items: i32,
    pub max_items: i32,
    pub min_extracts: i32,
    pub max_extracts: i32,
    pub min_spec_exits: i32,
    pub max_spec_exits: i32,
}

// =============================================================================
// CSM (Case Space Manager) Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct Csm {
    pub enable_cases: bool,
    pub enable_secure_cases: bool,
    pub custom_pocket: bool,
    pub pockets: Pockets,
    pub cases: Cases,
    pub secure_containers: SecureContainers,
    #[serde(rename = "EnableCSM")]
    pub enable_csm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Pockets {
    pub spec_g_keychain: bool,
    pub spec_simple_wallet: bool,
    #[serde(rename = "SpecWZWallet")]
    pub spec_wz_wallet: bool,
    pub spec_keycard_holder: bool,
    pub spec_keytool: bool,
    pub spec_injector_case: bool,
    pub spec_slots: i32,
    pub fourth_width: i32,
    pub fourth_height: i32,
    pub third_width: i32,
    pub third_height: i32,
    pub second_width: i32,
    pub second_height: i32,
    pub first_width: i32,
    pub first_height: i32,
}

impl Default for Pockets {
    fn default() -> Self {
        Self {
            spec_g_keychain: false,
            spec_simple_wallet: false,
            spec_wz_wallet: false,
            spec_keycard_holder: false,
            spec_keytool: false,
            spec_injector_case: false,
            spec_slots: 3,
            fourth_width: 1,
            fourth_height: 1,
            third_width: 1,
            third_height: 1,
            second_width: 1,
            second_height: 1,
            first_width: 1,
            first_height: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Cases {
    pub g_keychain: Case,
    pub keycard_holder_case: Case,
    pub injector_case: Case,
    pub holodilnick: Case,
    pub pistol_case: Case,
    pub documents_case: Case,
    pub keytool: Case,
    pub sicc_case: Case,
    pub thicc_weapon_case: Case,
    pub thicc_items_case: Case,
    pub medicine_case: Case,
    pub dogtag_case: Case,
    pub magazine_case: Case,
    pub ammunition_case: Case,
    pub weapon_case: Case,
    pub items_case: Case,
    pub grenade_case: Case,
    #[serde(rename = "WZWallet")]
    pub wz_wallet: Case,
    pub simple_wallet: Case,
    pub money_case: Case,
    pub lucky_scav: Case,
    pub streamer_case: Case,
    pub armor_plate_case: Case,
    pub keys_case: Case,
}

impl Default for Cases {
    fn default() -> Self {
        Self {
            g_keychain: Case {
                height: 2,
                width: 2,
                filter: false,
            },
            keycard_holder_case: Case {
                height: 3,
                width: 3,
                filter: false,
            },
            injector_case: Case {
                height: 3,
                width: 3,
                filter: false,
            },
            holodilnick: Case {
                height: 8,
                width: 8,
                filter: false,
            },
            pistol_case: Case {
                height: 3,
                width: 4,
                filter: false,
            },
            documents_case: Case {
                height: 4,
                width: 4,
                filter: false,
            },
            keytool: Case {
                height: 4,
                width: 4,
                filter: false,
            },
            sicc_case: Case {
                height: 5,
                width: 5,
                filter: false,
            },
            thicc_weapon_case: Case {
                height: 15,
                width: 6,
                filter: false,
            },
            thicc_items_case: Case {
                height: 14,
                width: 14,
                filter: false,
            },
            medicine_case: Case {
                height: 7,
                width: 7,
                filter: false,
            },
            dogtag_case: Case {
                height: 10,
                width: 10,
                filter: false,
            },
            magazine_case: Case {
                height: 7,
                width: 7,
                filter: false,
            },
            ammunition_case: Case {
                height: 7,
                width: 7,
                filter: false,
            },
            weapon_case: Case {
                height: 10,
                width: 5,
                filter: false,
            },
            items_case: Case {
                height: 8,
                width: 8,
                filter: false,
            },
            grenade_case: Case {
                height: 8,
                width: 8,
                filter: false,
            },
            wz_wallet: Case {
                height: 2,
                width: 2,
                filter: false,
            },
            simple_wallet: Case {
                height: 2,
                width: 2,
                filter: false,
            },
            money_case: Case {
                height: 7,
                width: 7,
                filter: false,
            },
            lucky_scav: Case {
                height: 14,
                width: 14,
                filter: false,
            },
            streamer_case: Case {
                height: 11,
                width: 7,
                filter: false,
            },
            armor_plate_case: Case {
                height: 12,
                width: 8,
                filter: false,
            },
            keys_case: Case {
                height: 7,
                width: 11,
                filter: false,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct Case {
    pub height: i32,
    pub width: i32,
    pub filter: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct SecureContainers {
    pub alpha: Case,
    pub beta: Case,
    pub epsilon: Case,
    pub gamma: Case,
    #[serde(rename = "GammaTUE")]
    pub gamma_tue: Case,
    pub kappa: Case,
    pub desecrated_kappa: Case,
    pub dev: Case,
    pub waist_pouch: Case,
}

impl Default for SecureContainers {
    fn default() -> Self {
        Self {
            alpha: Case {
                height: 2,
                width: 2,
                filter: false,
            },
            beta: Case {
                height: 2,
                width: 3,
                filter: false,
            },
            epsilon: Case {
                height: 2,
                width: 4,
                filter: false,
            },
            gamma: Case {
                height: 3,
                width: 3,
                filter: false,
            },
            gamma_tue: Case {
                height: 3,
                width: 3,
                filter: false,
            },
            kappa: Case {
                height: 4,
                width: 3,
                filter: false,
            },
            desecrated_kappa: Case {
                height: 4,
                width: 3,
                filter: false,
            },
            dev: Case {
                height: 3,
                width: 3,
                filter: false,
            },
            waist_pouch: Case {
                height: 2,
                width: 2,
                filter: false,
            },
        }
    }
}

// =============================================================================
// Scav Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Scav {
    #[serde(rename = "SCAVPockets")]
    pub scav_pockets: ScavPockets,
    pub hostile_bosses: bool,
    pub friendly_bosses: bool,
    pub car_base_standing: f64,
    pub scav_timer: i32,
    pub scav_custom_pockets: bool,
    pub scav_lab: bool,
    pub friendly_scavs: bool,
    pub hostile_scavs: bool,
    pub standing_friendly_kill: f64,
    #[serde(rename = "StandingPMCKill")]
    pub standing_pmc_kill: f64,
    pub health: Health,
    pub enable_scav_health: bool,
    pub enable_scav: bool,
    pub scav_stats: Stats,
    pub enable_stats: bool,
}

impl Default for Scav {
    fn default() -> Self {
        Self {
            scav_pockets: ScavPockets::default(),
            hostile_bosses: false,
            friendly_bosses: false,
            car_base_standing: 0.25,
            scav_timer: 900,
            scav_custom_pockets: false,
            scav_lab: false,
            friendly_scavs: false,
            hostile_scavs: false,
            standing_friendly_kill: -0.04,
            standing_pmc_kill: 0.01,
            health: Health {
                head: 35,
                chest: 85,
                stomach: 70,
                left_arm: 60,
                left_leg: 65,
                right_arm: 60,
                right_leg: 65,
            },
            enable_scav_health: false,
            enable_scav: false,
            scav_stats: Stats::default(),
            enable_stats: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct ScavPockets {
    pub fourth_height: i32,
    pub fourth_width: i32,
    pub third_height: i32,
    pub third_width: i32,
    pub second_height: i32,
    pub second_width: i32,
    pub first_height: i32,
    pub first_width: i32,
}

impl Default for ScavPockets {
    fn default() -> Self {
        Self {
            fourth_height: 1,
            fourth_width: 1,
            third_height: 1,
            third_width: 1,
            second_height: 1,
            second_width: 1,
            first_height: 1,
            first_width: 1,
        }
    }
}

// =============================================================================
// Bots Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Bots {
    #[serde(rename = "AIChance")]
    pub ai_chance: AiChance,
    #[serde(rename = "PMC")]
    pub pmc: BotDurability,
    #[serde(rename = "SCAV")]
    pub scav: BotDurability,
    pub boss: BotDurability,
    pub follower: BotDurability,
    pub rogue: BotDurability,
    pub raider: BotDurability,
    pub marksman: BotDurability,
    pub enable_bots: bool,
}

impl Default for Bots {
    fn default() -> Self {
        Self {
            ai_chance: AiChance::default(),
            pmc: BotDurability {
                armor_min: 90,
                armor_max: 100,
                weapon_min: 95,
                weapon_max: 100,
            },
            scav: BotDurability {
                armor_min: 0,
                armor_max: 50,
                weapon_min: 85,
                weapon_max: 100,
            },
            boss: BotDurability {
                armor_min: 85,
                armor_max: 100,
                weapon_min: 50,
                weapon_max: 100,
            },
            follower: BotDurability {
                armor_min: 90,
                armor_max: 100,
                weapon_min: 85,
                weapon_max: 100,
            },
            rogue: BotDurability {
                armor_min: 90,
                armor_max: 100,
                weapon_min: 80,
                weapon_max: 100,
            },
            raider: BotDurability {
                armor_min: 90,
                armor_max: 100,
                weapon_min: 80,
                weapon_max: 100,
            },
            marksman: BotDurability {
                armor_min: 90,
                armor_max: 100,
                weapon_min: 60,
                weapon_max: 100,
            },
            enable_bots: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct AiChance {
    pub kolontay_streets: i32,
    #[serde(rename = "KolontayGZ")]
    pub kolontay_gz: i32,
    pub force_partisan: bool,
    pub disable_weekly_boss: bool,
    pub partisan_customs: i32,
    pub partisan_shoreline: i32,
    pub partisan_woods: i32,
    pub partisan_lighthouse: i32,
    pub kaban: i32,
    pub tagilla_night: i32,
    pub trio_lighthouse: i32,
    pub trio_shoreline: i32,
    pub trio_woods: i32,
    pub zryachiy: i32,
    pub cultist_customs: i32,
    pub cultist_shoreline: i32,
    pub trio: i32,
    pub raider_lab: i32,
    pub raider_reserve: i32,
    pub cultist_factory: i32,
    pub cultist_woods: i32,
    pub cultist_ground_zero: i32,
    pub rogue: i32,
    pub tagilla: i32,
    pub shturman: i32,
    pub glukhar: i32,
    pub sanitar: i32,
    pub reshala: i32,
    pub killa: i32,
}

impl Default for AiChance {
    fn default() -> Self {
        Self {
            kolontay_streets: 30,
            kolontay_gz: 0,
            force_partisan: false,
            disable_weekly_boss: false,
            partisan_customs: 15,
            partisan_shoreline: 30,
            partisan_woods: 30,
            partisan_lighthouse: 30,
            kaban: 50,
            tagilla_night: 30,
            trio_lighthouse: 20,
            trio_shoreline: 20,
            trio_woods: 20,
            zryachiy: 100,
            cultist_customs: 20,
            cultist_shoreline: 15,
            trio: 20,
            raider_lab: 45,
            raider_reserve: 35,
            cultist_factory: 20,
            cultist_woods: 20,
            cultist_ground_zero: 44,
            rogue: 70,
            tagilla: 30,
            shturman: 50,
            glukhar: 50,
            sanitar: 30,
            reshala: 39,
            killa: 49,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
pub struct BotDurability {
    pub armor_min: i32,
    pub armor_max: i32,
    pub weapon_min: i32,
    pub weapon_max: i32,
}

// =============================================================================
// PMC Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Pmc {
    pub name_override: bool,
    #[serde(rename = "PMCChance")]
    pub pmc_chance: PmcChance,
    pub level_up_margin: i32,
    pub level_down_margin: i32,
    #[serde(rename = "PMCNameList")]
    pub pmc_name_list: String,
    pub names_enable: bool,
    pub chances_enable: bool,
    #[serde(rename = "PMCRatio")]
    pub pmc_ratio: i32,
    pub disable_low_level_pmc: bool,
    pub lootable_melee: bool,
    #[serde(rename = "EnablePMC")]
    pub enable_pmc: bool,
}

impl Default for Pmc {
    fn default() -> Self {
        Self {
            name_override: false,
            pmc_chance: PmcChance::default(),
            level_up_margin: 10,
            level_down_margin: 70,
            pmc_name_list: "Lacyway\r\nTrippyone\r\nssh\r\nDeadLeaves\r\nArchangel\r\nTheSparta\r\nSeion\r\nDOKDOR\r\nguidot\r\nGhostFenixx\r\nJanuary\r\nMorgan\r\nNimbul\r\nShiro\r\nTallan\r\nEkuland\r\nHustleHarder\r\nMissingTarget\r\nMrElmoEN\r\nNekoKami\r\nuprior\r\nVenican\r\nShynd\r\nCWX\r\nEreshkigal\r\nSenko\r\nChomp\r\nsptlaggy\r\nSerWolfik\r\nVolcano\r\nNexus4880\r\nFireHawk\r\nZ3R0\r\nRakTheGoose\r\nMorgan\r\nAssAssIn\r\nTabi\r\nG10rgos\r\nDaveyB0y\r\nFortis\r\nolli991\r\nKain187\r\nGamesB4Gains\r\nKiriko\r\nBiddinWar\r\n루퍼\r\n고라니\r\ncelebrutu\r\nogruby\r\nKWJimWails\r\nMaxomatic458\r\nNickMillion\r\nzartabulon\r\nzedramus\r\nslickboi\r\nNejurnia\r\nJuncker\r\nQuikstar\r\ntarkin\r\nTraveler\r\nBushtail\r\nRepublicanJesus\r\nMalecSP\r\n".to_string(),
            names_enable: false,
            chances_enable: false,
            pmc_ratio: 50,
            disable_low_level_pmc: false,
            lootable_melee: false,
            enable_pmc: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct PmcChance {
    #[serde(rename = "PMCNamePrefix")]
    pub pmc_name_prefix: i32,
    #[serde(rename = "PMCAllNamePrefix")]
    pub pmc_all_name_prefix: i32,
    #[serde(rename = "PMCLooseWep")]
    pub pmc_loose_wep: i32,
    pub hostile_same_pmc: i32,
    pub hostile_pmc: i32,
    #[serde(rename = "PMCWepEnhance")]
    pub pmc_wep_enhance: i32,
}

impl Default for PmcChance {
    fn default() -> Self {
        Self {
            pmc_name_prefix: 15,
            pmc_all_name_prefix: 5,
            pmc_loose_wep: 15,
            hostile_same_pmc: 85,
            hostile_pmc: 100,
            pmc_wep_enhance: 5,
        }
    }
}

// =============================================================================
// Custom Section
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct Custom {
    pub enable_custom: bool,
    #[serde(rename = "DisableSPTFriend")]
    pub disable_spt_friend: bool,
    pub disable_commando: bool,
    #[serde(rename = "DisablePMCMessages")]
    pub disable_pmc_messages: bool,
    #[serde(rename = "IDChanger")]
    pub id_changer: bool,
    pub flea_mult_id: String,
    #[serde(rename = "IDDefault")]
    pub id_default: String,
    #[serde(rename = "IDParent")]
    pub id_parent: String,
    #[serde(rename = "IDFilter")]
    pub id_filter: String,
    #[serde(rename = "IDPrice")]
    pub id_price: String,
    pub add_trader_assort: String,
    pub blacklist: String,
}

impl Default for Custom {
    fn default() -> Self {
        Self {
            enable_custom: false,
            disable_spt_friend: false,
            disable_commando: false,
            disable_pmc_messages: false,
            id_changer: false,
            flea_mult_id: "5780cf7f2459777de4559322:1.8".to_string(),
            id_default: String::new(),
            id_parent: String::new(),
            id_filter: String::new(),
            id_price: String::new(),
            add_trader_assort: String::new(),
            blacklist: String::new(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn default_config_round_trips_through_json() {
        let config = SvmConfig::default();
        let json = serde_json::to_string_pretty(&config).unwrap();
        let parsed: SvmConfig = serde_json::from_str(&json).unwrap();
        let json2 = serde_json::to_string_pretty(&parsed).unwrap();
        assert_eq!(json, json2);
    }

    #[test]
    fn empty_json_deserializes_to_defaults() {
        let config: SvmConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.raids.timeacceleration, 7);
        assert_eq!(config.raids.sandbox_access_level, 20);
        assert_eq!(config.player.health.head, 35);
        assert_eq!(config.player.health.chest, 85);
        assert!(!config.items.examine_keys);
    }

    #[test]
    fn partial_json_fills_missing_with_defaults() {
        let json = r#"{"Raids": {"RaidTime": 45}}"#;
        let config: SvmConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.raids.raid_time, 45);
        assert_eq!(config.raids.timeacceleration, 7); // default preserved
    }

    #[test]
    fn pascal_case_serialization() {
        let config = SvmConfig::default();
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("Raids").is_some());
        assert!(json.get("raids").is_none());
        let raids = json.get("Raids").unwrap();
        assert!(raids.get("RaidTime").is_some());
    }
}
