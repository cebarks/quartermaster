-- Migrate modsync.manage permission to convoy.manage and update triggers

-- Update existing permission assignments
UPDATE role_permissions SET permission = 'convoy.manage' WHERE permission = 'modsync.manage';

-- Drop old triggers
DROP TRIGGER IF EXISTS trg_role_permissions_permission_insert;
DROP TRIGGER IF EXISTS trg_role_permissions_permission_update;

-- Recreate triggers with convoy.manage instead of modsync.manage
CREATE TRIGGER trg_role_permissions_permission_insert
BEFORE INSERT ON role_permissions
BEGIN
    SELECT RAISE(ABORT, 'role_permissions.permission must be a known permission slug')
    WHERE NEW.permission NOT IN (
        'mods.install', 'mods.update', 'mods.remove', 'mods.disable',
        'convoy.manage', 'svm.edit', 'requests.resolve',
        'server.control', 'server.logs', 'server.metrics',
        'headless.manage', 'queue.manage', 'users.manage', 'settings.manage'
    );
END;

CREATE TRIGGER trg_role_permissions_permission_update
BEFORE UPDATE OF permission ON role_permissions
BEGIN
    SELECT RAISE(ABORT, 'role_permissions.permission must be a known permission slug')
    WHERE NEW.permission NOT IN (
        'mods.install', 'mods.update', 'mods.remove', 'mods.disable',
        'convoy.manage', 'svm.edit', 'requests.resolve',
        'server.control', 'server.logs', 'server.metrics',
        'headless.manage', 'queue.manage', 'users.manage', 'settings.manage'
    );
END;
