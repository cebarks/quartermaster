-- Migrate modsync.manage permission to convoy.manage and update triggers

-- Drop old triggers first, before updating the permission
DROP TRIGGER IF EXISTS trg_role_permissions_permission_insert;
DROP TRIGGER IF EXISTS trg_role_permissions_permission_update;

-- Update existing permission assignments (now that old triggers are gone)
UPDATE role_permissions SET permission = 'convoy.manage' WHERE permission = 'modsync.manage';

-- Recreate triggers with convoy.manage instead of modsync.manage
CREATE TRIGGER trg_role_permissions_permission_insert
BEFORE INSERT ON role_permissions
BEGIN
    SELECT RAISE(ABORT, 'role_permissions.permission must be a known permission slug')
    WHERE NEW.permission NOT IN (
        'mods.install', 'mods.update', 'mods.remove', 'mods.disable',
        'mods.config_edit',
        'convoy.manage', 'svm.edit', 'requests.resolve',
        'server.control', 'server.logs', 'server.metrics',
        'headless.manage', 'queue.manage', 'users.manage', 'settings.manage',
        'items.give', 'notes.edit', 'notes.manage'
    );
END;

CREATE TRIGGER trg_role_permissions_permission_update
BEFORE UPDATE OF permission ON role_permissions
BEGIN
    SELECT RAISE(ABORT, 'role_permissions.permission must be a known permission slug')
    WHERE NEW.permission NOT IN (
        'mods.install', 'mods.update', 'mods.remove', 'mods.disable',
        'mods.config_edit',
        'convoy.manage', 'svm.edit', 'requests.resolve',
        'server.control', 'server.logs', 'server.metrics',
        'headless.manage', 'queue.manage', 'users.manage', 'settings.manage',
        'items.give', 'notes.edit', 'notes.manage'
    );
END;
