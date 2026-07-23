use anyhow::Result;
use clap::ValueEnum;
use serde::Serialize;

use super::common::{find_unmanaged_mod_dirs, truncate_str, CliContext};

use crate::config::{FIKA_CLIENT_FORGE_ID, FIKA_SERVER_FORGE_ID};

const INFRASTRUCTURE_FORGE_IDS: &[i64] = &[FIKA_CLIENT_FORGE_ID, FIKA_SERVER_FORGE_ID];

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TreeMode {
    /// Show what each mod depends on
    Deps,
    /// Show what depends on each mod
    Rdeps,
}

#[derive(Serialize)]
struct ModEntry {
    name: String,
    version: String,
    forge_mod_id: Option<i64>,
    slug: Option<String>,
    file_count: usize,
    installed_at: String,
    updated_at: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    addons: Vec<AddonEntry>,
}

#[derive(Serialize)]
struct AddonEntry {
    name: String,
    version: String,
    forge_addon_id: i64,
    file_count: usize,
}

#[derive(Serialize)]
struct UnmanagedEntry {
    directory: String,
    file_count: usize,
}

#[derive(Serialize)]
struct ListOutput {
    infrastructure: Vec<ModEntry>,
    mods: Vec<ModEntry>,
    unmanaged: Vec<UnmanagedEntry>,
}

pub fn run(json: bool, tree: Option<TreeMode>, ctx: &CliContext) -> Result<()> {
    if let Some(mode) = tree {
        return run_tree(json, mode, ctx);
    }

    let installed_mods = ctx.db.list_mods()?;

    let all_tracked_files = ctx.db.get_all_tracked_files()?;
    let mut file_counts: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    let mut addon_file_counts: std::collections::HashMap<i64, usize> =
        std::collections::HashMap::new();
    for f in &all_tracked_files {
        if let Some(mid) = f.mod_id {
            *file_counts.entry(mid).or_default() += 1;
        }
        if let Some(aid) = f.addon_id {
            *addon_file_counts.entry(aid).or_default() += 1;
        }
    }

    // Fetch all addons and group by parent mod
    let all_addons = ctx.db.list_addons()?;
    let addons_by_parent: std::collections::HashMap<i64, Vec<_>> =
        all_addons
            .into_iter()
            .fold(std::collections::HashMap::new(), |mut map, addon| {
                map.entry(addon.parent_mod_id).or_default().push(addon);
                map
            });

    let mut infra_entries = Vec::new();
    let mut mod_entries = Vec::new();
    for m in &installed_mods {
        let addon_entries: Vec<AddonEntry> = addons_by_parent
            .get(&m.id)
            .map(|addons| {
                addons
                    .iter()
                    .map(|a| AddonEntry {
                        name: a.name.clone(),
                        version: a.version.clone(),
                        forge_addon_id: a.forge_addon_id,
                        file_count: addon_file_counts.get(&a.id).copied().unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let entry = ModEntry {
            name: m.name.clone(),
            version: m.version.clone(),
            forge_mod_id: m.forge_mod_id,
            slug: m.slug.clone(),
            file_count: file_counts.get(&m.id).copied().unwrap_or(0),
            installed_at: m.installed_at.clone(),
            updated_at: m.updated_at.clone(),
            addons: addon_entries,
        };
        if m.forge_mod_id
            .is_some_and(|id| INFRASTRUCTURE_FORGE_IDS.contains(&id))
        {
            infra_entries.push(entry);
        } else {
            mod_entries.push(entry);
        }
    }

    let (unmanaged_dirs, _) = find_unmanaged_mod_dirs(&ctx.dirs, &ctx.db)?;
    let unmanaged_entries: Vec<UnmanagedEntry> = unmanaged_dirs
        .into_iter()
        .map(|(dir, count)| UnmanagedEntry {
            directory: dir,
            file_count: count,
        })
        .collect();

    if json {
        let output = ListOutput {
            infrastructure: infra_entries,
            mods: mod_entries,
            unmanaged: unmanaged_entries,
        };
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // Infrastructure section
    if !infra_entries.is_empty() {
        println!("Infrastructure:");
        for entry in &infra_entries {
            println!("  {} {}", entry.name, entry.version);
        }
        println!();
    }

    // Table output
    if mod_entries.is_empty() && unmanaged_entries.is_empty() && infra_entries.is_empty() {
        println!("No mods installed and no unmanaged mods found.");
        return Ok(());
    }

    if !mod_entries.is_empty() {
        println!(
            "{:<30} {:<12} {:<8} {:<20}",
            "Name", "Version", "Files", "Installed"
        );
        println!("{}", "-".repeat(72));

        for entry in &mod_entries {
            let date = &entry.installed_at[..10.min(entry.installed_at.len())];
            println!(
                "{:<30} {:<12} {:<8} {:<20}",
                truncate_str(&entry.name, 29),
                entry.version,
                entry.file_count,
                date,
            );

            // Display nested addons with └─ prefix
            for addon in &entry.addons {
                println!(
                    "  └─ {:<26} {:<12} {:<8}",
                    truncate_str(&addon.name, 25),
                    addon.version,
                    addon.file_count,
                );
            }
        }
    }

    if !unmanaged_entries.is_empty() {
        println!("\nUnmanaged mods:");
        for entry in &unmanaged_entries {
            println!("  {} ({} files)", entry.directory, entry.file_count);
        }
        println!("\nManage them through the web UI or reinstall via Forge.");
    }

    Ok(())
}

// --- Dependency tree rendering ---

fn run_tree(json: bool, mode: TreeMode, ctx: &CliContext) -> Result<()> {
    let installed_mods = ctx.db.list_mods()?;
    let all_deps = ctx.db.get_all_dependencies()?;

    if json {
        let output = build_tree_json(&installed_mods, &all_deps, mode);
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        let output = render_tree_string(&installed_mods, &all_deps, mode);
        print!("{output}");
    }

    Ok(())
}

enum TreeChild {
    Installed(i64),
    Uninstalled { name: String },
}

fn format_mod_label(m: &crate::db::mods::InstalledMod) -> String {
    let mut label = format!("{} v{}", m.name, m.version);
    if m.disabled {
        label.push_str(" (disabled)");
    }
    label
}

fn render_tree_string(
    mods: &[crate::db::mods::InstalledMod],
    deps: &[crate::db::mods::ModDependency],
    mode: TreeMode,
) -> String {
    use std::collections::{HashMap, HashSet};
    use std::fmt::Write;

    let mod_by_id: HashMap<i64, &crate::db::mods::InstalledMod> =
        mods.iter().map(|m| (m.id, m)).collect();

    // Build adjacency list based on mode
    // deps mode: parent_id -> children it depends on
    // rdeps mode: parent_id -> children that depend on it
    let mut children: HashMap<i64, Vec<TreeChild>> = HashMap::new();

    for dep in deps {
        match mode {
            TreeMode::Deps => {
                let child = if let Some(dep_mod_id) = dep.depends_on_mod_id {
                    if mod_by_id.contains_key(&dep_mod_id) {
                        TreeChild::Installed(dep_mod_id)
                    } else {
                        TreeChild::Uninstalled {
                            name: dep
                                .depends_on_name
                                .clone()
                                .unwrap_or_else(|| "unknown".to_string()),
                        }
                    }
                } else {
                    TreeChild::Uninstalled {
                        name: dep
                            .depends_on_name
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                    }
                };
                children.entry(dep.mod_id).or_default().push(child);
            }
            TreeMode::Rdeps => {
                if let Some(dep_mod_id) = dep.depends_on_mod_id {
                    children
                        .entry(dep_mod_id)
                        .or_default()
                        .push(TreeChild::Installed(dep.mod_id));
                }
            }
        }
    }

    let mut sorted_mods: Vec<&crate::db::mods::InstalledMod> = mods.iter().collect();
    sorted_mods.sort_by_key(|a| a.name.to_lowercase());

    let mut out = String::new();
    for (i, m) in sorted_mods.iter().enumerate() {
        if i > 0 {
            writeln!(out).ok();
        }
        let label = format_mod_label(m);
        writeln!(out, "{label}").ok();

        if let Some(kids) = children.get(&m.id) {
            let mut visited = HashSet::new();
            visited.insert(m.id);
            print_children(&mut out, kids, &children, &mod_by_id, &mut visited, "");
        }
    }

    out
}

fn print_children(
    out: &mut String,
    kids: &[TreeChild],
    all_children: &std::collections::HashMap<i64, Vec<TreeChild>>,
    mod_by_id: &std::collections::HashMap<i64, &crate::db::mods::InstalledMod>,
    visited: &mut std::collections::HashSet<i64>,
    prefix: &str,
) {
    use std::fmt::Write;

    for (i, child) in kids.iter().enumerate() {
        let is_last = i == kids.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last {
            format!("{prefix}    ")
        } else {
            format!("{prefix}│   ")
        };

        match child {
            TreeChild::Uninstalled { name } => {
                writeln!(out, "{prefix}{connector}{name} (not installed)").ok();
            }
            TreeChild::Installed(mod_id) => {
                if !visited.insert(*mod_id) {
                    if let Some(m) = mod_by_id.get(mod_id) {
                        writeln!(
                            out,
                            "{prefix}{connector}{} v{} (circular)",
                            m.name, m.version
                        )
                        .ok();
                    }
                    continue;
                }

                if let Some(m) = mod_by_id.get(mod_id) {
                    let label = format_mod_label(m);
                    writeln!(out, "{prefix}{connector}{label}").ok();

                    if let Some(grandkids) = all_children.get(mod_id) {
                        print_children(
                            out,
                            grandkids,
                            all_children,
                            mod_by_id,
                            visited,
                            &child_prefix,
                        );
                    }
                }

                visited.remove(mod_id);
            }
        }
    }
}

// --- JSON tree output ---

#[derive(Serialize)]
struct TreeNodeJson {
    name: String,
    version: Option<String>,
    forge_mod_id: Option<i64>,
    slug: Option<String>,
    installed: bool,
    disabled: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dependencies: Vec<TreeNodeJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dependents: Vec<TreeNodeJson>,
}

fn build_tree_json(
    mods: &[crate::db::mods::InstalledMod],
    deps: &[crate::db::mods::ModDependency],
    mode: TreeMode,
) -> Vec<TreeNodeJson> {
    use std::collections::{HashMap, HashSet};

    let mod_by_id: HashMap<i64, &crate::db::mods::InstalledMod> =
        mods.iter().map(|m| (m.id, m)).collect();

    let mut children: HashMap<i64, Vec<TreeChild>> = HashMap::new();
    for dep in deps {
        match mode {
            TreeMode::Deps => {
                let child = if let Some(dep_mod_id) = dep.depends_on_mod_id {
                    if mod_by_id.contains_key(&dep_mod_id) {
                        TreeChild::Installed(dep_mod_id)
                    } else {
                        TreeChild::Uninstalled {
                            name: dep
                                .depends_on_name
                                .clone()
                                .unwrap_or_else(|| "unknown".to_string()),
                        }
                    }
                } else {
                    TreeChild::Uninstalled {
                        name: dep
                            .depends_on_name
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                    }
                };
                children.entry(dep.mod_id).or_default().push(child);
            }
            TreeMode::Rdeps => {
                if let Some(dep_mod_id) = dep.depends_on_mod_id {
                    children
                        .entry(dep_mod_id)
                        .or_default()
                        .push(TreeChild::Installed(dep.mod_id));
                }
            }
        }
    }

    let mut sorted_mods: Vec<&crate::db::mods::InstalledMod> = mods.iter().collect();
    sorted_mods.sort_by_key(|a| a.name.to_lowercase());

    sorted_mods
        .iter()
        .map(|m| {
            let mut visited = HashSet::new();
            visited.insert(m.id);
            build_json_node(m, &children, &mod_by_id, &mut visited, mode)
        })
        .collect()
}

fn build_json_node(
    m: &crate::db::mods::InstalledMod,
    all_children: &std::collections::HashMap<i64, Vec<TreeChild>>,
    mod_by_id: &std::collections::HashMap<i64, &crate::db::mods::InstalledMod>,
    visited: &mut std::collections::HashSet<i64>,
    mode: TreeMode,
) -> TreeNodeJson {
    let is_deps_mode = matches!(mode, TreeMode::Deps);
    let kids = all_children.get(&m.id);

    let child_nodes: Vec<TreeNodeJson> = kids
        .map(|kids| {
            kids.iter()
                .filter_map(|child| match child {
                    TreeChild::Uninstalled { name } => Some(TreeNodeJson {
                        name: name.clone(),
                        version: None,
                        forge_mod_id: None,
                        slug: None,
                        installed: false,
                        disabled: false,
                        dependencies: vec![],
                        dependents: vec![],
                    }),
                    TreeChild::Installed(mod_id) => {
                        if !visited.insert(*mod_id) {
                            return None;
                        }
                        let result = mod_by_id.get(mod_id).map(|child_mod| {
                            build_json_node(child_mod, all_children, mod_by_id, visited, mode)
                        });
                        visited.remove(mod_id);
                        result
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let (dependencies, dependents) = if is_deps_mode {
        (child_nodes, vec![])
    } else {
        (vec![], child_nodes)
    };

    TreeNodeJson {
        name: m.name.clone(),
        version: Some(m.version.clone()),
        forge_mod_id: m.forge_mod_id,
        slug: m.slug.clone(),
        installed: true,
        disabled: m.disabled,
        dependencies,
        dependents,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::db::mods::ModDependency;

    fn mock_mod(
        id: i64,
        name: &str,
        version: &str,
        forge_id: Option<i64>,
        disabled: bool,
    ) -> crate::db::mods::InstalledMod {
        crate::db::mods::InstalledMod {
            id,
            forge_mod_id: forge_id,
            forge_version_id: None,
            name: name.to_string(),
            slug: None,
            version: version.to_string(),
            installed_at: "2026-01-01".to_string(),
            updated_at: None,
            disabled,
            source: "forge".to_string(),
            source_url: None,
            group_id: None,
        }
    }

    #[test]
    fn build_tree_deps_mode() {
        let mods = vec![
            mock_mod(1, "Fika Server", "2.3.1", Some(2326), false),
            mock_mod(2, "Fika Client", "2.3.1", Some(2357), false),
            mock_mod(3, "SVM", "1.5.8", Some(1062), false),
        ];
        let deps = vec![ModDependency {
            id: 1,
            mod_id: 2,
            depends_on_mod_id: Some(1),
            depends_on_forge_id: Some(2326),
            depends_on_name: Some("Fika Server".to_string()),
            version_constraint: None,
        }];

        let output = render_tree_string(&mods, &deps, TreeMode::Deps);
        assert!(output.contains("Fika Client v2.3.1"));
        assert!(output.contains("└── Fika Server v2.3.1"));
        assert!(output.contains("SVM v1.5.8"));
    }

    #[test]
    fn build_tree_rdeps_mode() {
        let mods = vec![
            mock_mod(1, "Fika Server", "2.3.1", Some(2326), false),
            mock_mod(2, "Fika Client", "2.3.1", Some(2357), false),
        ];
        let deps = vec![ModDependency {
            id: 1,
            mod_id: 2,
            depends_on_mod_id: Some(1),
            depends_on_forge_id: Some(2326),
            depends_on_name: Some("Fika Server".to_string()),
            version_constraint: None,
        }];

        let output = render_tree_string(&mods, &deps, TreeMode::Rdeps);
        assert!(output.contains("Fika Server v2.3.1"));
        assert!(output.contains("└── Fika Client v2.3.1"));
    }

    #[test]
    fn tree_shows_disabled_annotation() {
        let mods = vec![mock_mod(1, "Mod A", "1.0", Some(100), true)];
        let deps = vec![];

        let output = render_tree_string(&mods, &deps, TreeMode::Deps);
        assert!(output.contains("Mod A v1.0 (disabled)"));
    }

    #[test]
    fn tree_shows_uninstalled_dep() {
        let mods = vec![mock_mod(1, "Mod A", "1.0", Some(100), false)];
        let deps = vec![ModDependency {
            id: 1,
            mod_id: 1,
            depends_on_mod_id: None,
            depends_on_forge_id: Some(999),
            depends_on_name: Some("Missing Mod".to_string()),
            version_constraint: None,
        }];

        let output = render_tree_string(&mods, &deps, TreeMode::Deps);
        assert!(output.contains("└── Missing Mod (not installed)"));
    }
}
