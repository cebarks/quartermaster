-- Roles table
CREATE TABLE IF NOT EXISTS roles (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    built_in     INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Role-permission junction table
CREATE TABLE IF NOT EXISTS role_permissions (
    role_id    INTEGER NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    permission TEXT NOT NULL,
    PRIMARY KEY (role_id, permission)
);

-- Seed built-in roles (OR IGNORE for idempotency on partial retry)
INSERT OR IGNORE INTO roles (name, display_name, built_in) VALUES ('admin', 'Admin', 1);
INSERT OR IGNORE INTO roles (name, display_name, built_in) VALUES ('moderator', 'Moderator', 1);
INSERT OR IGNORE INTO roles (name, display_name, built_in) VALUES ('player', 'Player', 1);

-- Admin: all permissions
INSERT OR IGNORE INTO role_permissions (role_id, permission)
SELECT r.id, p.permission
FROM roles r,
     (SELECT 'mods.install' AS permission
      UNION ALL SELECT 'mods.update'
      UNION ALL SELECT 'mods.remove'
      UNION ALL SELECT 'mods.disable'
      UNION ALL SELECT 'modsync.manage'
      UNION ALL SELECT 'svm.edit'
      UNION ALL SELECT 'requests.resolve'
      UNION ALL SELECT 'server.control'
      UNION ALL SELECT 'server.logs'
      UNION ALL SELECT 'server.metrics'
      UNION ALL SELECT 'headless.manage'
      UNION ALL SELECT 'queue.manage'
      UNION ALL SELECT 'users.manage'
      UNION ALL SELECT 'settings.manage') p
WHERE r.name = 'admin';

-- Moderator: everything except users.manage and settings.manage
INSERT OR IGNORE INTO role_permissions (role_id, permission)
SELECT r.id, p.permission
FROM roles r,
     (SELECT 'mods.install' AS permission
      UNION ALL SELECT 'mods.update'
      UNION ALL SELECT 'mods.remove'
      UNION ALL SELECT 'mods.disable'
      UNION ALL SELECT 'modsync.manage'
      UNION ALL SELECT 'svm.edit'
      UNION ALL SELECT 'requests.resolve'
      UNION ALL SELECT 'server.control'
      UNION ALL SELECT 'server.logs'
      UNION ALL SELECT 'server.metrics'
      UNION ALL SELECT 'headless.manage'
      UNION ALL SELECT 'queue.manage') p
WHERE r.name = 'moderator';

-- Player: no permissions (role exists, no rows in role_permissions)

-- Normalize existing role values (case-insensitive fix for any manual DB edits)
UPDATE users SET role = LOWER(role) WHERE role != LOWER(role);
