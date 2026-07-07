-- Update permission constraint triggers to include mods.config_edit
DROP TRIGGER IF EXISTS trg_role_permissions_permission_insert;
DROP TRIGGER IF EXISTS trg_role_permissions_permission_update;

CREATE TRIGGER trg_role_permissions_permission_insert
BEFORE INSERT ON role_permissions
BEGIN
    SELECT RAISE(ABORT, 'role_permissions.permission must be a known permission slug')
    WHERE NEW.permission NOT IN (
        'mods.install', 'mods.update', 'mods.remove', 'mods.disable',
        'mods.config_edit',
        'modsync.manage', 'svm.edit', 'requests.resolve',
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
        'modsync.manage', 'svm.edit', 'requests.resolve',
        'server.control', 'server.logs', 'server.metrics',
        'headless.manage', 'queue.manage', 'users.manage', 'settings.manage',
        'items.give', 'notes.edit', 'notes.manage'
    );
END;
