-- Prevent duplicate pending operations in the queue
-- Partial unique indexes ensure one pending operation per (mod/addon, action)

CREATE UNIQUE INDEX IF NOT EXISTS idx_pending_ops_mod_action
  ON pending_operations(forge_mod_id, action)
  WHERE item_type = 'mod' AND forge_mod_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_pending_ops_addon_action
  ON pending_operations(forge_addon_id, action)
  WHERE item_type = 'addon' AND forge_addon_id IS NOT NULL;
