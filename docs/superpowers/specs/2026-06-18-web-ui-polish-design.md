# Web UI Polish — Design Spec

**Date:** 2026-06-18
**Approach:** CSS-first polish (no new dependencies, no build step, no CDN)
**Scope:** Visual refinement + UX improvements across all pages, with focus on Dashboard and Status

## 1. Dashboard Redesign

### Summary Stat Row

Add a 3-column CSS grid row at the top of the dashboard with stat cards:

- **Mods** — count of installed mods, colored left border (green)
- **Queue** — count of pending operations, colored left border (yellow when pending, green when empty)
- **Server** — live status dot + "Running"/"Down" + latency, colored left border matching state

Each stat card links to its respective page. Uses `.stat-card` CSS class with `border-left: 3px solid <color>`.

### Template Data

The dashboard handler needs two additional fields:
- `server_reachable: bool`
- `server_latency_ms: Option<u64>`

Loaded via a new HTMX partial (`/api/dashboard/server-status`) that returns the stat card HTML. This avoids slowing the initial page load and reuses the existing server health check logic from the status handler.

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
- Background tint: subtle green when up, subtle red when down
- Status dot with CSS `box-shadow` glow when online

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
- Increase `border-radius` from 6px to 8px
- Increase padding from 1.25rem to 1.5rem

### Buttons

- Transition both `background` and `border-color`
- Add visible `:focus-visible` ring for keyboard navigation
- Review text contrast on `.btn-success` and `.btn-warning`

### Tables

- Increase row padding from 0.5rem to 0.65rem
- Reduce border opacity for subtler row separation
- Slightly more visible hover highlight

### Typography

- `h1` gets a bottom border (`1px solid var(--border)`) with padding-bottom as a visual anchor
- Better spacing between page title and first content card

### Badges

- Semi-transparent backgrounds instead of solid fills
- Pattern: `background: rgba(color, 0.15); color: <color>; border: 1px solid rgba(color, 0.3)`
- Increased border-radius for pill shape

### Links

- Add subtle underline on hover for distinguishability in dense content

### Empty States

- Consistent centered styling with muted text
- Suggest actions when relevant

## 4. Inline SVG Icons

### Approach

Small set of inline SVGs embedded via an Askama macro file (`partials/icons.html`). Each icon is a `{% macro icon_name() %}` that emits an inline `<svg>`. No icon library, no font loading, no external files.

### Icon Inventory (~10 icons)

| Icon | Usage |
|------|-------|
| home | Nav: Dashboard |
| package | Nav: Mods |
| list | Nav: Queue |
| activity | Nav: Status |
| play | Server: Start button |
| refresh | Server: Restart button, Update button |
| stop (square) | Server: Stop button |
| download | Install button |
| trash | Remove button |
| x | Cancel button |

All icons are 16x16 or 20x20, single-path SVGs. Negligible size impact.

## 5. UX Improvements

### Toast / Flash Notifications

- Use the existing `{% block flash %}` in `base.html`
- After POST redirects, display a toast message at the top of the page
- CSS animation: slide down on appear, fade out after 3 seconds
- Color-coded: green (success), red (error), yellow (warning)
- Implementation: use `actix-session` to store flash messages (key: `flash_message`, `flash_type`) in handlers before redirect. Template reads and clears on render. No additional dependencies needed.

### Confirmation Dialogs

Add `onsubmit="return confirm(...)"` to forms for:
- Stop Server
- Restart Server
- Cancel Queue Operation
- Apply All Queue
- Update All Mods

### Loading States

- Style `.htmx-indicator` as a visible pulsing dot or small spinner
- Ensure `hx-indicator` attributes are set on all HTMX-triggered elements

### Form Feedback

- Disable submit buttons during HTMX requests to prevent double-submit
- Uses HTMX's built-in `htmx-request` class to toggle disabled state via CSS

### Improved Empty States

- Queue page: centered "All clear" message with subtle check icon
- Dashboard mods: actionable text pointing to the Mods page
- Consistent styling pattern across all empty states

## Non-Goals

- No CSS framework (Tailwind, Bootstrap, etc.)
- No additional JS beyond HTMX
- No mobile-responsive breakpoints (desktop admin panel)
- No new pages or routes
- No changes to authentication or authorization logic

## Files Affected

### Templates (modify)
- `templates/base.html` — flash block, icon import
- `templates/dashboard.html` — stat row, refined mod table
- `templates/status.html` — hero card layout, control button grouping
- `templates/partials/status_detail.html` — hero + grid layout, status dot glow
- `templates/partials/nav.html` — icons before link text
- `templates/mods/list.html` — button icons, confirm dialogs, empty state
- `templates/mods/detail.html` — button icons, badge polish
- `templates/queue.html` — confirm dialogs, empty state, badge polish
- `templates/login.html` — minor spacing/polish
- `templates/register.html` — minor spacing/polish
- `templates/mods/partials/update_badges.html` — softer badge style

### Templates (new)
- `templates/partials/icons.html` — SVG icon macros

### CSS (modify)
- `src/assets/style.css` — all global polish changes, new component classes

### Rust handlers (modify)
- `src/web/handlers/dashboard.rs` — new HTMX partial endpoint for server status stat card
- `src/web/handlers/*.rs` (auth, mods, queue, server) — add flash messages to session before redirects
- `src/web/mod.rs` — register new `/api/dashboard/server-status` route
