-- Add multi-use invite code support.
-- max_uses NULL = unlimited, 1 = single-use (backward compat default).
ALTER TABLE invite_codes ADD COLUMN max_uses INTEGER DEFAULT 1;
ALTER TABLE invite_codes ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0;

-- Backfill: existing used codes should reflect their single use.
UPDATE invite_codes SET use_count = 1 WHERE used_by IS NOT NULL;
