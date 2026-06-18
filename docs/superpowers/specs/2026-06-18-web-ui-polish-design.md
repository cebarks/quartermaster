# Web UI Polish — Design Spec

**Date:** 2026-06-18
**Approach:** CSS-first polish (no new dependencies, no build step, no CDN)
**Scope:** Visual refinement + UX improvements across all pages, with focus on Dashboard and Status

## 1. Dashboard Redesign

### Summary Stat Row

Add a 3-column CSS grid row at the top of the dashboard with stat cards:

- **Mods** — count of installed mods, colored left border (green). Entire card is an `<a>` wrapping to `/mods`.
- **Queue** — count of pending operations, colored left border (yellow when pending, green when empty). Links to `/queue`.
- **Server** — live status dot + "Running"/"Down" + latency, colored left border matching state. Links to `/status`.

Uses `.stat-card` CSS class with `border-left: 3px solid <color>`.

### Server Status in Stat Row

The server stat card renders inline with a placeholder state ("Checking...") and loads its content via HTMX:

```html
<a href="/status" class="stat-card" style="border-left-color: var(--border)">
  <div class="stat-label">Server</div>
  <div hx-get="/api/dashboard/server-status" hx-trigger="load" hx-swap="innerHTML">
    <span class="text-muted text-sm">Checking...</span>
  </div>
</a>
```

The CSS grid is always 3 columns (`grid-template-columns: 1fr 1fr 1fr`), so the placeholder card occupies its space from initial render — no layout shift.

**New HTMX partial:** `/api/dashboard/server-status`
- **Handler:** `dashboard::server_status_partial` in `src/web/handlers/dashboard.rs`
- **Template:** `templates/partials/dashboard_server_status.html` (new file) — renders the status dot, "Running"/"Down" text, and latency
- **Template struct:** `DashboardServerStatusTemplate { reachable: bool, latency_ms: Option<u64> }`
- **Health check logic:** Extract `build_health_report` (or a lighter `check_server_reachable`) from `status.rs` into a shared function (make it `pub` or move to a shared module). The dashboard partial only needs reachability + latency, not the full `HealthReport`.

### Mod Table Refinements

- Softer update badge: semi-transparent background (`rgba(240,192,64,0.15)`) with colored text/border instead of solid fill
- More row padding for readability
- Better empty states with actionable messaging

### Pending Operations / Unmanaged Mods

Keep existing cards but apply the global card polish (shadows, radius, padding).

## 2. Status Page Redesign

### Server Health — Hero Card

Promote the SPT Server section to a visually distinct hero card:

- Larger status text (server state + latency on one line)
- Address and version as secondary metadata below
- Background tint: subtle green when up, subtle red when down (`background: rgba(78,204,163,0.06)` / `rgba(233,69,96,0.06)`)
- Status dot with CSS `box-shadow: 0 0 6px rgba(78,204,163,0.5)` glow when online

### Mods + Integrity — Secondary Grid

Display the Mods and Integrity sections in a 2-column grid below the hero:

- Stat-card style with colored left borders (matching dashboard stat cards)
- Numbers prominent, labels muted
- Badges only appear when non-zero (updates available, missing files, etc.)

### Server Control Buttons

- Visually grouped below the hero card
- Add confirmation dialogs on Stop and Restart (currently missing)

## 3. Global CSS Polish

### Cards

- Add `box-shadow: 0 2px 8px rgba(0,0,0,0.2)` for depth
- Add a card-specific `border-radius: 8px` (override, not changing `--radius` which remains 6px for inputs/buttons/alerts)
- Increase padding from 1.25rem to 1.5rem

### Buttons

- Transition both `background` and `border-color`
- Add `:focus-visible` ring: `outline: 2px solid var(--accent); outline-offset: 2px`
- Improve `.btn-success` and `.btn-warning` text contrast: use `#0d1117` (near-black) instead of `#1a1a2e`

### Tables

- Increase row padding from 0.5rem to 0.65rem
- Reduce border opacity for subtler row separation: `border-color: rgba(42,42,74,0.6)` instead of solid `var(--border)`
- Slightly more visible hover highlight: `rgba(255,255,255,0.05)` instead of `0.03`

### Typography

- `h1` gets a bottom border (`1px solid var(--border)`) with `padding-bottom: 0.5rem` as a visual anchor
- Add `margin-bottom: 1.25rem` between page title and first content card

### Badges

- Semi-transparent backgrounds instead of solid fills
- Pattern: `background: rgba(color, 0.15); color: <color>; border: 1px solid rgba(color, 0.3)`
- `border-radius: 10px` for pill shape

### Links

- Add `text-decoration: underline` on hover for distinguishability in dense content

### Empty States

- New `.empty-state` class: centered text, `padding: 2rem`, `color: var(--text-muted)`
- Suggest actions when relevant (e.g., "No mods installed. Install one from the Mods page.")

## 4. Inline SVG Icons

### Approach

Small set of inline SVGs embedded via an Askama macro file (`partials/icons.html`). Each icon is a `{% macro icon_name() %}` that emits an inline `<svg>`. No icon library, no font loading, no external files.

Every template that uses icons must add `{% import "partials/icons.html" as icons %}` at the top. Templates needing the import:
- `templates/partials/nav.html` (nav icons)
- `templates/mods/list.html` (install, update, remove icons)
- `templates/mods/detail.html` (update, remove icons)
- `templates/queue.html` (cancel icon)
- `templates/status.html` (server control icons)
- `templates/partials/status_detail.html` (status indicators)

### Icon Inventory (~12 icons)

| Icon | Usage |
|------|-------|
| home | Nav: Dashboard |
| package | Nav: Mods |
| list | Nav: Queue |
| activity | Nav: Status |
| play | Server: Start button |
| refresh | Server: Restart button, Update/Update All buttons |
| stop (square) | Server: Stop button |
| download | Install button |
| trash | Remove button |
| x | Cancel button |
| log-out | Nav: Logout button |
| check | Empty state: "All clear" indicator |

All icons are 16x16, single-path SVGs with `fill="none" stroke="currentColor" stroke-width="2"` (Lucide-style). Negligible size impact.

## 5. UX Improvements

### Toast / Flash Notifications

**Architecture:** Askama templates are compile-time structs — they cannot read the session directly. Flash messages flow through the handler layer:

1. **Setting flash (in POST handlers before redirect):** Call a helper `set_flash(&session, "message", "success")` which stores `flash_message` and `flash_type` in the session.
2. **Reading flash (in GET handlers that render pages):** Call `take_flash(&session) -> Option<(String, String)>` which reads and clears the flash from the session in one step.
3. **Passing to template:** Every page-rendering template struct gets `flash: Option<FlashMessage>` where `FlashMessage` is a simple struct `{ message: String, flash_type: String }`.
4. **Rendering:** `base.html`'s `{% block flash %}` gets a default implementation that checks `flash` and renders the toast. Child templates inherit this without needing to override the block — Askama supports default block content in the base template as long as `flash` is a field on the child struct.

**Helper location:** `src/web/flash.rs` (new module) with `set_flash()`, `take_flash()`, and `FlashMessage` struct.

**Which handlers set flash:**
- `mods::install_mod` — "Mod queued for install" (success)
- `mods::update_mod` / `update_all_mods` — "Update queued" (success)
- `mods::remove_mod` — "Mod queued for removal" (success)
- `queue::cancel_op` — "Operation cancelled" (success)
- `queue::apply_queue` — "Queue applied" (success) or error message (error)
- `server::start` / `stop` / `restart` — "Server started/stopped/restarted" (success)

**Which handlers read flash (all page-rendering handlers):**
- `dashboard::dashboard`
- `mods::list_mods`
- `mods::mod_detail`
- `queue::queue_page`
- `status::status_page`

**Login/register pages:** These already use their own `error: Option<String>` field rendered as `.alert-error`. Flash toasts do NOT apply to auth pages — they use a separate, simpler error display pattern that already works.

**Toast CSS:**
- `.toast` class with `animation: toast-in 0.3s, toast-out 0.3s 3s forwards`
- `@keyframes toast-in` — slide down from top
- `@keyframes toast-out` — fade out
- Not dismissible on click or hover-pausable (polish-level, not production notification system)

### Confirmation Dialogs

Add `onsubmit="return confirm(...)"` to forms for:
- Stop Server
- Restart Server
- Cancel Queue Operation
- Apply All Queue
- Update All Mods

Note: this uses inline `confirm()` which is vanilla JS, not HTMX. This is consistent with existing patterns (mod Remove already uses `onsubmit="return confirm(...)"`).

### Loading States

- Style `.htmx-indicator` with a CSS pulsing animation
- Add `hx-indicator` attributes to:
  - Dashboard update check span (already uses HTMX, indicator implicit)
  - Status page content div (already uses HTMX)
  - New server status stat card on dashboard

### Form Feedback

- Disable submit buttons during HTMX requests to prevent double-submit
- Uses HTMX's built-in `htmx-request` class: `.htmx-request .btn { pointer-events: none; opacity: 0.6; }`

### Improved Empty States

- Queue page: centered "All clear" with check icon
- Dashboard mods: actionable text pointing to the Mods page
- Mod list: actionable text for admins
- Consistent `.empty-state` class across all empty states

## Out of Scope

- CSS framework (Tailwind, Bootstrap, etc.)
- Mobile-responsive breakpoints (desktop admin panel)
- Changes to authentication or authorization logic
- Styled HTML error pages (currently plain text from `WebError`) — future improvement
- `templates/mods/partials/dependency_tree.html` — badge CSS changes apply automatically via global CSS; no template changes needed

## Files Affected

### Templates (modify)
- `templates/base.html` — flash block default content with toast rendering
- `templates/dashboard.html` — stat row, refined mod table, flash field
- `templates/status.html` — hero card layout, control button grouping, confirm dialogs
- `templates/partials/status_detail.html` — hero + grid layout, status dot glow
- `templates/partials/nav.html` — icons before link text, logout icon
- `templates/mods/list.html` — button icons, confirm dialogs, empty state
- `templates/mods/detail.html` — button icons
- `templates/queue.html` — confirm dialogs, empty state, badge polish
- `templates/login.html` — minor spacing/polish
- `templates/register.html` — minor spacing/polish
- `templates/mods/partials/update_badges.html` — softer badge style (via global CSS)

### Templates (new)
- `templates/partials/icons.html` — SVG icon macros
- `templates/partials/dashboard_server_status.html` — HTMX partial for server stat card content

### CSS (modify)
- `src/assets/style.css` — all global polish changes, new component classes (`.stat-card`, `.stat-card-grid`, `.empty-state`, `.toast`, `.hero-card`, `.status-grid`)

### Rust (new)
- `src/web/flash.rs` — `FlashMessage` struct, `set_flash()`, `take_flash()` helpers

### Rust (modify)
- `src/web/mod.rs` — register `/api/dashboard/server-status` route, add `flash` module
- `src/web/handlers/dashboard.rs` — add `server_status_partial` handler, add flash reading to `dashboard`
- `src/web/handlers/mods.rs` — add flash to `list_mods`, `mod_detail`; set flash in install/update/remove handlers
- `src/web/handlers/queue.rs` — add flash to `queue_page`; set flash in cancel/apply handlers
- `src/web/handlers/status.rs` — add flash to `status_page`; set flash in start/stop/restart; make health check logic public/shared
- `src/web/handlers/server.rs` — set flash in start/stop/restart handlers
