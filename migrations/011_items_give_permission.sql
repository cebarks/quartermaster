-- Add items.give to permission constraint triggers
DROP TRIGGER IF EXISTS trg_role_permissions_permission_insert;
DROP TRIGGER IF EXISTS trg_role_permissions_permission_update;

CREATE TRIGGER trg_role_permissions_permission_insert
BEFORE INSERT ON role_permissions
BEGIN
    SELECT RAISE(ABORT, 'role_permissions.permission must be a known permission slug')
    WHERE NEW.permission NOT IN (
        'mods.install', 'mods.update', 'mods.remove', 'mods.disable',
        'modsync.manage', 'svm.edit', 'requests.resolve',
        'server.control', 'server.logs', 'server.metrics',
        'headless.manage', 'queue.manage', 'users.manage', 'settings.manage',
        'items.give'
    );
END;

CREATE TRIGGER trg_role_permissions_permission_update
BEFORE UPDATE OF permission ON role_permissions
BEGIN
    SELECT RAISE(ABORT, 'role_permissions.permission must be a known permission slug')
    WHERE NEW.permission NOT IN (
        'mods.install', 'mods.update', 'mods.remove', 'mods.disable',
        'modsync.manage', 'svm.edit', 'requests.resolve',
        'server.control', 'server.logs', 'server.metrics',
        'headless.manage', 'queue.manage', 'users.manage', 'settings.manage',
        'items.give'
    );
END;

-- Give admin role the items.give permission
INSERT OR IGNORE INTO role_permissions (role_id, permission)
SELECT id, 'items.give' FROM roles WHERE name = 'admin';
