# Template Deduplication Plan

## Baseline

- 30 clones, 5605 duplicated tokens (10.79% of 51970 total tokens)
- 72 files analyzed

## Refactoring Items

### 1. request_card.html <-> requests.html (1,238 tokens -- LARGEST)

**What's duplicated**: The entire request card body (lines 5-117 of `request_card.html`) is duplicated inside the `{% for r in &requests %}` loop body in `requests.html` (lines 67-179). These are ~113 lines of identical template code for rendering a single mod request card with voting, resolve forms, install buttons, etc.

**HTMX analysis**: `request_card.html` IS an HTMX swap target -- the `RequestCardTemplate` is rendered by handlers for vote, resolve, and install_from_request endpoints. They use `hx-target="#request-{{ r.request.id }}" hx-swap="outerHTML"` to replace individual cards. So it must remain independently renderable.

**Approach**: Have `requests.html` include `request_card.html` inside the loop instead of duplicating the card body. The scoping works because `request_card.html` uses `r` as the variable and `requests.html` also iterates with `{% for r in &requests %}`, so the variable name matches. The only difference is:
- `request_card.html` has a wrapping `<div id="request-{{ r.request.id }}">` and an optional toast message at the top (lines 1-4)
- `requests.html` has the same wrapping `<div id="request-{{ r.request.id }}">` but NO message toast

**Solution**: Extract the card BODY (the `<div class="request-card card">...</div>`) into a new partial `templates/mods/partials/request_card_body.html`. Both `request_card.html` and `requests.html` include it. Both keep their own outer wrapper divs. `request_card.html` keeps its toast/message handling. The `RequestCardTemplate` struct already has all fields needed by `request_card_body.html` (via `r`, `csrf_token`, `user`). The `RequestsTabTemplate` also has those same fields.

**Risk**: Low. Variables are compatible. Include scoping in Askama inherits parent variables.

### 2. admin/partials/user_row.html <-> admin/partials/users.html (1,067 tokens)

**What's duplicated**: The entire `user_row.html` template (146 lines) is the table row, and `users.html` has the same row content inline within its `{% for (u, profile) in users %}` loop.

**HTMX analysis**: `user_row.html` IS an HTMX swap target -- `UserRowTemplate` is rendered by admin handlers for role change, disable, reset-password, link-profile, and delete operations, all targeting `closest tr` or `#admin-content`.

**Key differences**:
- `user_row.html` uses `u` directly (from `UserRowTemplate.u`) + has `row_message` and `reset_link` fields
- `users.html` destructures as `{% for (u, profile) in users %}` giving same `u` variable
- `users.html` doesn't have `row_message` or `reset_link` (initial render doesn't need them)
- `user_row.html` line 3-5: `{% if let Some(msg) = row_message %}` toast -- this won't render in users.html since there's no `row_message` variable

**Solution**: Replace the inline row in `users.html` with `{% include "admin/partials/user_row.html" %}`. But there's a problem: `users.html` doesn't have `row_message`, `reset_link`, `roles`, or `available_profiles` as individual variables -- they're in the `UsersTabTemplate` struct but under different names/structures.

Let me check the `UsersTabTemplate` struct more carefully.

Actually, looking at the templates:
- `user_row.html` uses: `u`, `profile`, `current_user_id`, `csrf_token`, `reset_link`, `row_message`, `roles`, `available_profiles`
- `users.html` uses: `users` (iterated), `current_user_id`, `csrf_token`, `roles`, `available_profiles`

In `users.html`, within the loop `{% for (u, profile) in users %}`, the variables `u` and `profile` are available, plus the parent template variables. So `current_user_id`, `csrf_token`, `roles`, `available_profiles` are all available.

The missing variables in `users.html` context are `row_message` and `reset_link`. In Askama, if a template includes another template, ALL variables from the including template's struct must be available. Since `users.html` is rendered by a struct that doesn't have `row_message` or `reset_link`, we can't just include `user_row.html` directly.

**Revised approach**: Extract the common row body into a new partial `templates/admin/partials/user_row_body.html` that contains the shared `<td>` cells content (without the `<tr>` wrapper and without the toast/reset_link parts). Then:
- `user_row.html` renders the `<tr>`, toast, includes body, and reset_link
- `users.html` renders the `<tr>` and includes body

Actually, this is more complex than useful. The toast and reset_link are woven into specific cells. The first `<td>` in `user_row.html` has the toast, and the last `<td>` has the reset_link -- these aren't separate sections easily extracted.

**Revised approach 2**: Make `users.html` use `{% include "admin/partials/user_row.html" %}` inside the loop. This requires adding `row_message` and `reset_link` to the `UsersTabTemplate`. Since these are `Option` types, we set them to `None` for the initial full-page render (they're only populated on HTMX responses).

Wait -- Askama `{% include %}` doesn't work that way. The included template references the parent template's struct fields. So if `users.html` is rendered by `UsersTabTemplate` which doesn't have `row_message` or `reset_link`, the include will fail at compile time because those variables aren't available.

The correct fix: Add `row_message: Option<String>` and `reset_link: Option<String>` to `UsersTabTemplate`. But that doesn't work either -- there's a single `row_message` and `reset_link` on the struct, but we need per-row values. Actually, in the loop context, `user_row.html` references `row_message` which would refer to a struct field, not a loop-scoped variable.

**Conclusion**: This duplication cannot be cleanly resolved with `{% include %}` because the per-row template (`user_row.html`) has extra fields (`row_message`, `reset_link`) that are only meaningful in the HTMX-response context and have no place in the full-table context.

**ACCEPTED DUPLICATION** -- The `user_row.html` exists specifically as an HTMX partial with extra state (toast message, reset link) that doesn't apply when rendering the full table.

### 3. clients/list.html <-> clients/partials/status.html (768 tokens, 2 clones)

**What's duplicated**: The entire client table (empty state + table with headers/rows) is present in both `list.html` (lines 35-126) and `status.html` (lines 1-92). The `status.html` partial is an HTMX swap target (refreshed every 5s via `hx-get="/quma/api/headless/status"`).

**Key differences**:
- `list.html` iterates `{% for client in clients %}`, `status.html` also uses `{% for client in clients %}`
- `list.html` renders players as `{{ client.players.join(", ") }}`, while `status.html` renders them as linked profile names: `<a href="/quma/profiles/{{ player }}">{{ player }}</a>`
- The `status.html` empty state message is identical to `list.html`

**Solution**: Since `list.html` is now just a redirect (the handler returns `SeeOther` to `/quma/settings?tab=headless`), the duplication in `list.html` is actually dead code. However, the template file still exists and still has a `ClientListTemplate` struct comment. Let me verify...

Actually, looking at the handler: `client_list` returns a redirect. But `list.html` is still a full template with `{% extends "base.html" %}`. It might be referenced elsewhere or kept as a fallback. Since it's effectively dead code, I'll mark this as accepted and focus on the `settings/clients.html` <-> `status.html` overlap.

**ACCEPTED DUPLICATION** -- `list.html` is dead code (handler redirects). The `status.html` partial is the canonical HTMX swap target. The settings page includes a different but structurally similar table inline.

### 4. raids/partials/recent.html <-> stats.html (427 tokens, 2 clones)

**What's duplicated**: The recent raids table in `stats.html` (lines 103-137) is almost identical to `raids/partials/recent.html` (lines 1-35). Similarly, the active raids table in `stats.html` (lines 34-52) matches `raids/partials/active.html` (lines 1-19).

**Key differences**:
- `stats.html` uses `raid_stats.recent_raids` while `recent.html` uses `recent_raids`
- `stats.html` uses `active_raids` (same name as `active.html`)

**HTMX analysis**: Both `active.html` and `recent.html` ARE HTMX swap targets:
- Active raids: `hx-get="/quma/api/raids/active"` triggers on SSE events
- Recent raids: `hx-get="/quma/api/raids/recent"` triggers on SSE events

**Solution for active raids**: `stats.html` can use `{% include "raids/partials/active.html" %}` directly since the variable name `active_raids` matches. The `StatsPageTemplate` already has `active_raids: Vec<(Raid, String)>`.

**Solution for recent raids**: Can't directly include because `stats.html` uses `raid_stats.recent_raids` but the partial expects `recent_raids`. We need to either:
a) Add a `recent_raids` field to `StatsPageTemplate` that aliases `raid_stats.recent_raids`, or
b) Accept the duplication

**Approach**: Add `recent_raids: Vec<(Raid, String)>` to `StatsPageTemplate` populated from `raid_stats.recent_raids`, then include the partial. This is a clean refactor.

**Risk**: Low. Variable types match. Need to update the Rust struct.

### 5. clients/list.html <-> settings/clients.html (378 tokens, 2 clones)

Similar to item 3. The settings/clients.html page has its own inline client table that structurally overlaps with list.html. Since settings/clients.html is `{% include %}`d into settings.html, it uses the `SettingsTemplate` struct's fields (`headless_clients`, `headless_converging`, `headless_target_count`).

The `status.html` partial uses `clients` not `headless_clients`. Can't directly include without a variable name change or adapter.

**ACCEPTED DUPLICATION** -- Different variable names across different template contexts. The settings page embeds the client table with additional configuration controls, making it a different component.

### 6. clients/detail.html <-> clients/list.html (358 tokens, 3 clones)

The `ContainerStatus`, `ClientHealth`, and `EHeadlessStatus` match blocks are duplicated across detail.html, list.html, and status.html. These are the `{% match %}` blocks for rendering status badges.

**Solution**: Extract three small macros into a shared file (e.g., `templates/clients/partials/badges.html`):
- Container status badge macro
- Client health badge macro
- Fika status badge macro

Actually, Askama macros must be defined with `{% macro name(args) %}...{% endmacro %}` and imported. This would work well here.

**Risk**: Low. Straightforward macro extraction.

### 7. mods/detail.html <-> mods/partials/addon_list.html (266 tokens)

**What's duplicated**: The addon table in `detail.html` (lines 163-206) is very similar to `addon_list.html` (lines 1-49).

**Key differences**:
- `detail.html` uses `user.can("mods.disable")` / `user.can("mods.remove")`, while `addon_list.html` uses pre-computed `can_disable` / `can_remove` booleans
- `detail.html` uses `{% for addon in &addons %}`, `addon_list.html` uses `{% for addon in addons %}`
- `addon_list.html` has `class="table"` on the table tag, `detail.html` doesn't

**HTMX analysis**: `addon_list.html` is an HTMX swap target rendered by `list_addons_partial()`.

**Solution**: Have `detail.html` use `{% include "mods/partials/addon_list.html" %}`. This requires:
1. Adding `can_disable` and `can_remove` fields to `ModDetailTemplate`
2. Adjusting the `addons` reference syntax

Actually, the include would reference the parent struct's fields. `ModDetailTemplate` has `user` and `addons` but not `can_disable`/`can_remove`. We'd need to add those fields. And the `&addons` vs `addons` difference matters in Askama iteration.

This is feasible but requires Rust struct changes. The bigger concern: `detail.html` wraps the addon table in more context (the "Addons (X)" header, the search section, the empty state). The `addon_list.html` partial IS the table itself. So `detail.html` would include `addon_list.html` in the middle of its addon section.

**Approach**: Add `can_disable: bool` and `can_remove: bool` to `ModDetailTemplate`, populate from `user.can()` checks in the handler. Change `addon_list.html` to use `&addons` for iteration consistency. Then replace the duplicated table in `detail.html` with `{% include "mods/partials/addon_list.html" %}`.

**Risk**: Medium. Need to verify the `&addons` vs `addons` iteration difference doesn't cause issues.

### 8. install_search_results.html <-> search_results.html (215 tokens, 3 clones)

**What's duplicated**: Error display, iteration structure, and Fika compatibility badges.

**Key differences**: These are fundamentally different templates despite structural similarity:
- `install_search_results.html` is for addon search (clicking selects a mod to install)
- `search_results.html` is for request search (clicking selects a mod to request)
- Different `data-*` attributes, different click handlers, different styling details
- `search_results.html` has an embedded `<script>` for click handling

**ACCEPTED DUPLICATION** -- These serve different UX flows with different behaviors. Extracting a shared partial would create a coupling that makes future changes harder. The overlap is structural, not semantic.

## Summary of Actions

| # | Pair | Tokens | Action |
|---|------|--------|--------|
| 1 | request_card <-> requests | 1,238 | Extract `request_card_body.html`, include in both |
| 2 | user_row <-> users | 1,067 | ACCEPTED -- HTMX partial with extra state |
| 3 | clients/list <-> status | 768 | ACCEPTED -- list.html is dead code |
| 4 | raids partials <-> stats | 427 | Include `active.html` in stats; add `recent_raids` field + include `recent.html` |
| 5 | clients/list <-> settings/clients | 378 | ACCEPTED -- different variable names/contexts |
| 6 | clients/detail <-> list <-> status | 358 | Extract badge macros |
| 7 | mods/detail <-> addon_list | 266 | Include addon_list.html in detail; add fields to struct |
| 8 | install_search_results <-> search_results | 215 | ACCEPTED -- different UX flows |

**Expected impact**: Items 1, 4, 6, 7 should eliminate ~2,000-2,500 tokens of duplication.
