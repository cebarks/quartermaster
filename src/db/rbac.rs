use std::collections::HashSet;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    ModsInstall,
    ModsUpdate,
    ModsRemove,
    ModsDisable,
    ModsyncManage,
    SvmEdit,
    RequestsResolve,
    ServerControl,
    ServerLogs,
    ServerMetrics,
    HeadlessManage,
    QueueManage,
    UsersManage,
    SettingsManage,
}

impl Permission {
    pub const ALL: &[Permission] = &[
        Permission::ModsInstall,
        Permission::ModsUpdate,
        Permission::ModsRemove,
        Permission::ModsDisable,
        Permission::ModsyncManage,
        Permission::SvmEdit,
        Permission::RequestsResolve,
        Permission::ServerControl,
        Permission::ServerLogs,
        Permission::ServerMetrics,
        Permission::HeadlessManage,
        Permission::QueueManage,
        Permission::UsersManage,
        Permission::SettingsManage,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Permission::ModsInstall => "mods.install",
            Permission::ModsUpdate => "mods.update",
            Permission::ModsRemove => "mods.remove",
            Permission::ModsDisable => "mods.disable",
            Permission::ModsyncManage => "modsync.manage",
            Permission::SvmEdit => "svm.edit",
            Permission::RequestsResolve => "requests.resolve",
            Permission::ServerControl => "server.control",
            Permission::ServerLogs => "server.logs",
            Permission::ServerMetrics => "server.metrics",
            Permission::HeadlessManage => "headless.manage",
            Permission::QueueManage => "queue.manage",
            Permission::UsersManage => "users.manage",
            Permission::SettingsManage => "settings.manage",
        }
    }

    pub fn from_slug(s: &str) -> Option<Permission> {
        match s {
            "mods.install" => Some(Permission::ModsInstall),
            "mods.update" => Some(Permission::ModsUpdate),
            "mods.remove" => Some(Permission::ModsRemove),
            "mods.disable" => Some(Permission::ModsDisable),
            "modsync.manage" => Some(Permission::ModsyncManage),
            "svm.edit" => Some(Permission::SvmEdit),
            "requests.resolve" => Some(Permission::RequestsResolve),
            "server.control" => Some(Permission::ServerControl),
            "server.logs" => Some(Permission::ServerLogs),
            "server.metrics" => Some(Permission::ServerMetrics),
            "headless.manage" => Some(Permission::HeadlessManage),
            "queue.manage" => Some(Permission::QueueManage),
            "users.manage" => Some(Permission::UsersManage),
            "settings.manage" => Some(Permission::SettingsManage),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Permission::ModsInstall => "Install Mods",
            Permission::ModsUpdate => "Update Mods",
            Permission::ModsRemove => "Remove Mods",
            Permission::ModsDisable => "Disable Mods",
            Permission::ModsyncManage => "Manage NarcoNet",
            Permission::SvmEdit => "Edit Server Config",
            Permission::RequestsResolve => "Resolve Mod Requests",
            Permission::ServerControl => "Control Server",
            Permission::ServerLogs => "View Logs",
            Permission::ServerMetrics => "View Metrics",
            Permission::HeadlessManage => "Manage Headless Clients",
            Permission::QueueManage => "Manage Queue",
            Permission::UsersManage => "Manage Users",
            Permission::SettingsManage => "Manage Settings",
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Permission::ModsInstall
            | Permission::ModsUpdate
            | Permission::ModsRemove
            | Permission::ModsDisable => "Mods",
            Permission::ModsyncManage | Permission::SvmEdit | Permission::RequestsResolve => {
                "Configuration"
            }
            Permission::ServerControl
            | Permission::ServerLogs
            | Permission::ServerMetrics
            | Permission::HeadlessManage => "Server",
            Permission::QueueManage => "Operations",
            Permission::UsersManage | Permission::SettingsManage => "Administration",
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct RoleRecord {
    pub id: i64,
    pub name: String,
    pub display_name: String,
    pub built_in: bool,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct RoleWithPermissions {
    pub role: RoleRecord,
    pub permissions: HashSet<Permission>,
}

impl fmt::Display for RoleRecord {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.display_name)
    }
}

/// Named struct for template use (avoids tuple indexing in Askama)
#[derive(Debug, Clone)]
pub struct PermissionInfo {
    pub slug: &'static str,
    pub display_name: &'static str,
    pub category: &'static str,
    pub checked: bool,
}

/// Validate a custom role name slug: [a-z0-9-], 1-32 chars, not a built-in name.
pub fn validate_role_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > 32 {
        return Err("Role name must be 1-32 characters");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("Role name may only contain lowercase letters, digits, and hyphens");
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err("Role name cannot start or end with a hyphen");
    }
    if matches!(name, "admin" | "moderator" | "player") {
        return Err("Cannot use a built-in role name");
    }
    Ok(())
}
