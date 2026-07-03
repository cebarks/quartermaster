-- Enforce users.role references a valid role name via trigger
-- (trigger-based instead of FK recreation for migration safety)

CREATE TRIGGER IF NOT EXISTS trg_users_role_insert
BEFORE INSERT ON users
BEGIN
    SELECT RAISE(ABORT, 'users.role must reference an existing role in the roles table')
    WHERE NEW.role NOT IN (SELECT name FROM roles);
END;

CREATE TRIGGER IF NOT EXISTS trg_users_role_update
BEFORE UPDATE OF role ON users
BEGIN
    SELECT RAISE(ABORT, 'users.role must reference an existing role in the roles table')
    WHERE NEW.role NOT IN (SELECT name FROM roles);
END;

-- Enforce role_permissions.permission is a known permission slug.
-- ponytail: allowlist must be updated when adding new Permission variants to rbac.rs

CREATE TRIGGER IF NOT EXISTS trg_role_permissions_permission_insert
BEFORE INSERT ON role_permissions
BEGIN
    SELECT RAISE(ABORT, 'role_permissions.permission must be a known permission slug')
    WHERE NEW.permission NOT IN (
        'mods.install', 'mods.update', 'mods.remove', 'mods.disable',
        'modsync.manage', 'svm.edit', 'requests.resolve',
        'server.control', 'server.logs', 'server.metrics',
        'headless.manage', 'queue.manage', 'users.manage', 'settings.manage'
    );
END;

CREATE TRIGGER IF NOT EXISTS trg_role_permissions_permission_update
BEFORE UPDATE OF permission ON role_permissions
BEGIN
    SELECT RAISE(ABORT, 'role_permissions.permission must be a known permission slug')
    WHERE NEW.permission NOT IN (
        'mods.install', 'mods.update', 'mods.remove', 'mods.disable',
        'modsync.manage', 'svm.edit', 'requests.resolve',
        'server.control', 'server.logs', 'server.metrics',
        'headless.manage', 'queue.manage', 'users.manage', 'settings.manage'
    );
END;
