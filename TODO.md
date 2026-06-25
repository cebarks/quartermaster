# TODO

## Status
- no action to remove untracked directories (displayed but not actionable)
- untracked file list only shows directory-level summary, not individual files

### Raids
- `src/db/raids.rs` has ~20 stale `#[allow(dead_code)]` annotations — all items are now actively used by handlers/raid_tracker

### Stash
- UX needs rework
- currency items (USD, EUR) displayed as roubles instead of as currency balances at the top of the page next to stash rouble count
