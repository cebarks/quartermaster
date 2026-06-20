# Changelog

All notable changes to Quartermaster will be documented in this file.

## [Unreleased]

### Bug Fixes

- Add contents:read permission to publish-crate workflow([40364d8](https://github.com/cebarks/quartermaster/commit/40364d8ff90c4b82b14a067deaf9c5b56a7cf6d7))
- Restrict CI push trigger to main to prevent duplicate runs (#69)([27bd47e](https://github.com/cebarks/quartermaster/commit/27bd47e8e1d0171f71f951f8da6f8ffeedce274d))

### Documentation

- Add setup rework design spec([d3ed01d](https://github.com/cebarks/quartermaster/commit/d3ed01d6d61d639ec1cdd3b53c9aecf7b86bef3e))
- Update setup rework spec with review feedback([a29c9ab](https://github.com/cebarks/quartermaster/commit/a29c9ab0c1bd73545dfcdf9ecf3fcded8b28b1b0))
- Add setup rework implementation plan([24bc51a](https://github.com/cebarks/quartermaster/commit/24bc51a2abc3c27773862c64c4364b17e56d918a))
- Add ModSync integration design spec([418e1eb](https://github.com/cebarks/quartermaster/commit/418e1eba8f097796845f48d7e50686e64b0e45ef))
- Address review feedback on ModSync integration spec([3e70bc0](https://github.com/cebarks/quartermaster/commit/3e70bc08a193d3dacb81e67a55beefc765d66f44))

### Features

- Add server_transition flag to AppState and ServerStateChanged SSE event([a0d0d96](https://github.com/cebarks/quartermaster/commit/a0d0d969853232fbd6c639d96d1d1b8e0f359ca8))
- Extend ServerHealth with transition and started_at fields([37f35df](https://github.com/cebarks/quartermaster/commit/37f35df26c5765176756e85bc057c8b598cc9654))
- Show intermediate server states and uptime on status page (#66)([c7d0cfb](https://github.com/cebarks/quartermaster/commit/c7d0cfb728a2470e10d8322dcc49e20b24e13bd8))
- Rework setup to bootstrap new servers (#67)([7b0a2bf](https://github.com/cebarks/quartermaster/commit/7b0a2bf9c8fce10b8a3b3b51ec32c2795170e488))
- Wire up server restart during client convergence (#70)([ff5b611](https://github.com/cebarks/quartermaster/commit/ff5b6110b01ef87e2e759f60098b5e4cb95d5aec))

### Miscellaneous

- Remove docs (#71)([ca38f57](https://github.com/cebarks/quartermaster/commit/ca38f572b319fe4479bb308236633cf561e0cea9))

### Testing

- Add DB unit tests and Forge client mock HTTP tests (#19, #20) (#65)([a7baf1a](https://github.com/cebarks/quartermaster/commit/a7baf1a79dca3257bd36a7a4ea12d033075b5f83))
## [0.1.0] - 2026-06-20

### Bug Fixes

- Address code review findings — security, API correctness, schema alignment([98f3fe8](https://github.com/cebarks/quartermaster/commit/98f3fe8ca02b4237db225a152c67addc59c751c2))
- Truncate_str off-by-one — nth(max) not nth(max-1)([24c1d94](https://github.com/cebarks/quartermaster/commit/24c1d94e0ba99d8862ee827b8fe7685bab8987f1))
- Replace unwrap with proper error in apply_single_update([241f7eb](https://github.com/cebarks/quartermaster/commit/241f7eb50420d07a47447caefbc425714f8ae984))
- Add ON DELETE CASCADE to depends_on_mod_id FK, recursive reverse deps([4fb1955](https://github.com/cebarks/quartermaster/commit/4fb195508d78cc484d375595e189f8e262734d22))
- Address review findings — atomic update, error handling, query determinism([314a148](https://github.com/cebarks/quartermaster/commit/314a148e19f60e16ef85870f8df36de081d8f476))
- Reverse dep check in queue drain, error context for apply ops([74073f2](https://github.com/cebarks/quartermaster/commit/74073f222af49410005a2b27224ba298cb9b775e))
- Handle shallow paths and hash errors in integrity check([690685a](https://github.com/cebarks/quartermaster/commit/690685a845012d338833de269ca6e78a7b99c752))
- Stop drain ordering, request timeouts, partial drain error reporting([2672085](https://github.com/cebarks/quartermaster/commit/2672085f0e5ac9857ae53ef3d755083ff8b83620))
- Update SPT directory detection for 4.0.13 Linux layout([c61d385](https://github.com/cebarks/quartermaster/commit/c61d38590000f023e491303ecc7c17ddd9d3f618))
- Update Forge API client for v0 query param format and response shapes([187bece](https://github.com/cebarks/quartermaster/commit/187bece5d50a85d26c2d7f3c1fa7d7a805f24c4d))
- Add should_queue() check to update_all_mods handler([b48bbbd](https://github.com/cebarks/quartermaster/commit/b48bbbdc26adf9684f3effdce7d71e17d26f86e7))
- Final review — add update op to apply_queue, replace unwrap with error returns, clean warnings([06d8625](https://github.com/cebarks/quartermaster/commit/06d8625d849720d2690ce238f061286356645779))
- Eliminate route shadowing by merging scopes, add per-handler admin checks([ae2f268](https://github.com/cebarks/quartermaster/commit/ae2f268f9c4a7045f55b7eb9082db4375cc0186a))
- Rewrite apply_queue with proper error handling and extract-before-delete([531bed8](https://github.com/cebarks/quartermaster/commit/531bed8dd5512186992d280144652c7a2f23f83f))
- Auth handler hardening — password validation, invite race, blocking IO([ff95204](https://github.com/cebarks/quartermaster/commit/ff952047ae9b9743d846f707a7679cc3164bf663))
- Address all review findings — config perms, health dedup, templates, clippy([082ddbf](https://github.com/cebarks/quartermaster/commit/082ddbf5ee914b75af84d5c1f0e2564b3724c738))
- Move assets and /api routes before catch-all auth scope([a322c77](https://github.com/cebarks/quartermaster/commit/a322c77f4606c358c8fd1c01f28369910463120c))
- Remove guarded unwrap and stale doc comment in setup.rs([8440e75](https://github.com/cebarks/quartermaster/commit/8440e75455ec6e32ba0c0babd963aab86bbc0617))
- Invite URL uses localhost when bound to 0.0.0.0, remove duplicate doc comment([6431ec9](https://github.com/cebarks/quartermaster/commit/6431ec9b190b1074bce005e2fd92a10e40c61a80))
- Register_submit now correctly records user_id on invite after creation([b2f27ae](https://github.com/cebarks/quartermaster/commit/b2f27ae23f43e5cdbb35c18daebc8d9f22635f9f))
- Session fixation, path traversal fail-open, missing admin auth on API partials([93bc87d](https://github.com/cebarks/quartermaster/commit/93bc87d7dac8dfb4c62caa164195c867cda7076c))
- Replace naive JSONC comment stripping with json_comments crate([9cbdbcf](https://github.com/cebarks/quartermaster/commit/9cbdbcf14d71e22f61b8ee5b8bb8b577998cc836))
- Eliminate flaky env var test failures with temp-env([ba31945](https://github.com/cebarks/quartermaster/commit/ba319456219dd8d89b4cbdc9a8e121a85fe07639))
- Use temp_env for remaining unsafe env var tests in config and init([03b23ac](https://github.com/cebarks/quartermaster/commit/03b23ac93612184f74afe0063af5fb5405605f2e))
- Add SPT compat warning for explicit version fallback, add load success message([02201b6](https://github.com/cebarks/quartermaster/commit/02201b6f0e9a32645336893a7c84d6e7bffe9972))
- Log podman is_running errors instead of swallowing them([5bfebb4](https://github.com/cebarks/quartermaster/commit/5bfebb42190269102e2c656055a891bf12864507))
- Address review findings — nav hover underline, h1 border in flex containers, toast collapse, queue error detail([7205964](https://github.com/cebarks/quartermaster/commit/7205964d11171b918552135b9dd8a0f47fc481dc))
- Render error pages with styled HTML template([50831b5](https://github.com/cebarks/quartermaster/commit/50831b5b954312dc7e309a3473321a9bec097c63))
- Convert mod handlers to async background tasks([ec205f6](https://github.com/cebarks/quartermaster/commit/ec205f6c8d9324f1281a1a8855375aede0c78a26))
- Address Task 4 review findings([78417ee](https://github.com/cebarks/quartermaster/commit/78417ee8b085e4b6a6ae0d7f57b9c960381813cc))
- Restore tracing middleware and info! log reverted by Task 5([0463238](https://github.com/cebarks/quartermaster/commit/0463238e5b4f47dac27705f91cfd84f424b022d7))
- Show flash errors for server control failures instead of raw errors([39c412f](https://github.com/cebarks/quartermaster/commit/39c412f97926904fb5a661efe7e88960dca4b4fe))
- Address final review findings([1356c8c](https://github.com/cebarks/quartermaster/commit/1356c8c5e9ac88b4d66d4505b890043d5e1b56a5))
- Don't hold DB lock during archive extraction([7f557c7](https://github.com/cebarks/quartermaster/commit/7f557c7b7064581c82f0c6a2db13986187be5778))
- Exclude managed mod dirs from unmanaged mods list([bca6ba1](https://github.com/cebarks/quartermaster/commit/bca6ba1f78b844b8431026c7d18454d888ab6070))
- Address final review findings([732d35b](https://github.com/cebarks/quartermaster/commit/732d35bdcd29ce441dffe46d03bd78e8a35b4751))
- Auth level and cache invalidation ordering in mod list partials([4f82581](https://github.com/cebarks/quartermaster/commit/4f82581bd43823e76e795da6a1a71383d4fe748b))
- SSE extension parent scope, OOB td parsing, and update version verification([4014557](https://github.com/cebarks/quartermaster/commit/4014557c1c30b16d4f28536a5032a2ac2b2d2c48))
- Verify update candidates against SPT version in dashboard badge([1ad1eb6](https://github.com/cebarks/quartermaster/commit/1ad1eb6519a844b4d5307dafd4e1aae40cc95b10))
- Pick newest compatible version instead of oldest from Forge API([f663bb1](https://github.com/cebarks/quartermaster/commit/f663bb1d2bddd82d0e42e931b8eef323455b8ac2))
- Remove unused Cli import from server.rs([c523369](https://github.com/cebarks/quartermaster/commit/c523369c60be7a4ec20362126cc4b0d3e19bd866))
- Server create works without existing SPT install([be6da97](https://github.com/cebarks/quartermaster/commit/be6da975b48b2d210ab7acce5e715d99c69ce707))
- Add logging to auth middleware error paths([fe25eef](https://github.com/cebarks/quartermaster/commit/fe25eef2108ea8e08f6f051d5a102170f7222414))
- Address review findings in auth middleware and login handler([e98bfc6](https://github.com/cebarks/quartermaster/commit/e98bfc664d3c3a3bd442b7f6057c39ed59487e65))
- Timestamp format, flash message, and disabled user check in password reset([bd58ce9](https://github.com/cebarks/quartermaster/commit/bd58ce9a6ebd7a7bbb0c433418bb8894e0e483d9))
- HTMX orphan row, hx-include scope, full reset URL, disabled user check, profile parse logging([d5cd0cf](https://github.com/cebarks/quartermaster/commit/d5cd0cf418902c86261b44435e2612fd683d0cb3))
- Critical bugs from adversarial review([4e4335d](https://github.com/cebarks/quartermaster/commit/4e4335d0937d45e4e98db69c6401d3f4b0011a35))
- Add tag trigger and crates.io publish to release workflow([12634e3](https://github.com/cebarks/quartermaster/commit/12634e3384847894925037f4e7ee28cf443e94b0))
- Installed tab navigation and reject comment field([367d563](https://github.com/cebarks/quartermaster/commit/367d5636428b5cab66db5ed01f6648bb407b7782))
- Move OptionalExtension import to top of requests.rs([560fc90](https://github.com/cebarks/quartermaster/commit/560fc9089a4c3b840ad737fecbd47bad084ee972))
- Critical bugs from adversarial review([2d61af0](https://github.com/cebarks/quartermaster/commit/2d61af00e06de6f11beedc0ab14e203d343ecde1))
- Persist client count to config after web scale operation (#33)([d0df18e](https://github.com/cebarks/quartermaster/commit/d0df18e74b47978ab253afa977de81b3d2de953c))
- Spawn client restarts as independent tasks (#63)([570d04e](https://github.com/cebarks/quartermaster/commit/570d04e6fad89f79257c37fc17ab6ac42761bcaf))
- Filter SPT core files from integrity check (#34)([e2ea890](https://github.com/cebarks/quartermaster/commit/e2ea8909ca40a8013f651987e8cebd048d4c361d))
- Use shared SELinux label for install_dir mount (#35)([e9b9419](https://github.com/cebarks/quartermaster/commit/e9b941919d5d9fdffb52e45f78163b72da4e13f2))
- Unify converging flag on AtomicBool (#36)([6039d8e](https://github.com/cebarks/quartermaster/commit/6039d8eccf4fcffcec6c820687c0428c2d9bd1d1))
- Regenerate cargo-dist release workflow to match config([52630d2](https://github.com/cebarks/quartermaster/commit/52630d28dde283573b9232cdde9fe17f5cd27a3b))
- Persist client scale to config file (#5)([7720e80](https://github.com/cebarks/quartermaster/commit/7720e80b1dc73fb328c92bf3b7969d3243d0023d))
- Correlate CLI status with per-client PROFILE_ID (#38)([1260452](https://github.com/cebarks/quartermaster/commit/12604524c9ee9b1502da83dc618f95cf3d4263c4))
- Assign PROFILE_ID env var to client containers during convergence([2a1217f](https://github.com/cebarks/quartermaster/commit/2a1217fc53e74229a35b3e917247707243deeb83))

### CI/CD

- Add GitHub Actions CI pipeline (fmt, clippy, test, audit)([80db6c8](https://github.com/cebarks/quartermaster/commit/80db6c84fc0f0e3936b2b37192057f0c77bf724b))
- Add cargo-dist release workflow([9435737](https://github.com/cebarks/quartermaster/commit/94357372658a346ee1912b2feb5a07fb20ed28fc))
- Add crates.io publishing to release workflow([986ae25](https://github.com/cebarks/quartermaster/commit/986ae256f9086e32d457bd0677c3bd1e0697f20d))

### Documentation

- Add implementation plan for quartermaster v1([9624a04](https://github.com/cebarks/quartermaster/commit/9624a04f5dd7da801ade5c26a845492ea6bb02ab))
- Add Phase 2 core CLI plan, track superpowers docs in repo([d14386e](https://github.com/cebarks/quartermaster/commit/d14386ee42dee4278b22d8f955dc9212af8c8923))
- Address 10 review findings in Phase 2 plan([bf0a44f](https://github.com/cebarks/quartermaster/commit/bf0a44fc36e140bc714351bb1a4355809ff7a983))
- Add web UI polish design spec([b626d70](https://github.com/cebarks/quartermaster/commit/b626d7056c989349403d501d7e8c19a2d1d50556))
- Address review findings in web UI polish spec([4602d55](https://github.com/cebarks/quartermaster/commit/4602d555e81bfc5074e4058cdead31cc5c761cd7))
- Add web UI polish implementation plan([34e13cd](https://github.com/cebarks/quartermaster/commit/34e13cd6f3b430bb15fca3e4241fcb00cd629edc))
- Add CLAUDE.md with codebase guidance for Claude Code([b8c502a](https://github.com/cebarks/quartermaster/commit/b8c502ade3150bd1d5bfdcbdd76637489d84277e))
- Document QUMA_AUTO_START_SERVER env var([5dbeaae](https://github.com/cebarks/quartermaster/commit/5dbeaae1eac3cb848967354a3ef5b99419a6f9b0))
- CI/CD & release automation design spec([d9de016](https://github.com/cebarks/quartermaster/commit/d9de016fb012c9683b2267b53ab7401d50196f0a))
- CI/CD & release automation implementation plan([d33e3dd](https://github.com/cebarks/quartermaster/commit/d33e3dd6afd693b43d8e6c89d077633d6ffb4303))
- CI/CD & release automation implementation plan([acfb9ba](https://github.com/cebarks/quartermaster/commit/acfb9ba6cf432295e6bf15654140a31defa7f0b8))
- CI/CD & release automation design spec([8e0812b](https://github.com/cebarks/quartermaster/commit/8e0812b3a42a7ab4d048461f09f84cc751d359ee))

### Features

- Phase 1 foundation — project scaffold, config, SPT detection, DB, Forge client, archive handling([2839ccf](https://github.com/cebarks/quartermaster/commit/2839ccf071437383ded98071c38151deeced6272))
- Add init command and shared CLI context([1254433](https://github.com/cebarks/quartermaster/commit/1254433cf8c9b0cab7e43ccaa7a9bee782aab1c5))
- Add install command with dependency resolution and Forge download([feb29df](https://github.com/cebarks/quartermaster/commit/feb29df81197f85ebd5ab48ab26c12ff9ca61e8c))
- Add remove command with reverse dependency checking([650c2b7](https://github.com/cebarks/quartermaster/commit/650c2b7b453bb8b5561e138fa687854f244aadb2))
- Add update command with Forge API update checking([c12a047](https://github.com/cebarks/quartermaster/commit/c12a04761a8bd813d1b848bafea7e17f574c3c7f))
- Add list and check commands for mod status reporting([cd96830](https://github.com/cebarks/quartermaster/commit/cd968303407f7d4772085846f360728d4765aa72))
- Add track command to associate unmanaged mods with Forge entries([c048fd6](https://github.com/cebarks/quartermaster/commit/c048fd64c68189317682d8ccaaac4c9cc29a2b5c))
- Podman integration, SPT server client, and unified server detection([4b96b3f](https://github.com/cebarks/quartermaster/commit/4b96b3f2bd5d1c4e30b775a09ac9087777558d77))
- Change queue system with apply command, queue integration in install/update/remove([952d520](https://github.com/cebarks/quartermaster/commit/952d520a24edd46d0ef584c74ef0a0550f4b4981))
- Health checks and status command with integrity verification and SPT compat check([27a6e3e](https://github.com/cebarks/quartermaster/commit/27a6e3e89a17cb562418c325829e4fba8e6a63ce))
- Server lifecycle commands — start, stop, restart, logs with queue drain([485a4b0](https://github.com/cebarks/quartermaster/commit/485a4b0c2bc2f858e19972b962b5c9ec6e7a9534))
- Add 7z archive support for mod detection and extraction([239298b](https://github.com/cebarks/quartermaster/commit/239298b5df4f3a212d305ec3f4938f18b822c7a0))
- Actix-web server foundation with static assets, base template, and serve command([3892e8f](https://github.com/cebarks/quartermaster/commit/3892e8fd0a1c7d745530d1d59b10493a42be8844))
- Auth system with login, register, logout, session middleware, and rate limiting([14b5bf8](https://github.com/cebarks/quartermaster/commit/14b5bf837e94698d31ad9bfe78c1e8d3957b83f1))
- Dashboard, mod list/detail pages, install/update/remove handlers, HTMX partials([a1c8a5e](https://github.com/cebarks/quartermaster/commit/a1c8a5e3a4871b09eff1aedc81fcd58f6d4d2b64))
- Queue management, server status, and server control pages with HTMX auto-refresh([b94e349](https://github.com/cebarks/quartermaster/commit/b94e349aa05027f1fdb53b7818e52093626f0fc0))
- Add config and invite CLI commands([fab84bb](https://github.com/cebarks/quartermaster/commit/fab84bba25281df5085780b812937f7300769c03))
- Add generate systemd command([e270034](https://github.com/cebarks/quartermaster/commit/e270034a3fd8cf4ac1c18bc3ba51bcb31be5ed69))
- Add setup command with guided Fika setup flow([a312588](https://github.com/cebarks/quartermaster/commit/a312588f765f1fbcd0b2cecf3a8cfc12891b82e6))
- Wire loadedServerMods into health checks([bbe03c6](https://github.com/cebarks/quartermaster/commit/bbe03c67bc9795e446b9366572003cbdf531a7ed))
- Wire explicit version argument for quma install([7078b42](https://github.com/cebarks/quartermaster/commit/7078b427aba6b1d6375db666eba53a920e140b68))
- Add rate limiting on /login and /register([52aae37](https://github.com/cebarks/quartermaster/commit/52aae37e513587bee29ac6d43b696d91df8a88f2))
- Add inline SVG icon macros (Lucide-style, 12 icons)([dbbc6e5](https://github.com/cebarks/quartermaster/commit/dbbc6e541ffdd29e8a2a5f9363dcd06d4f2071ec))
- Add flash message module (set_flash, take_flash, FlashMessage)([5d2ad05](https://github.com/cebarks/quartermaster/commit/5d2ad051beee273d69bb443949b5db0c807a4f2d))
- Wire flash toast notifications through all handlers and templates([b9b94a1](https://github.com/cebarks/quartermaster/commit/b9b94a13fccd615c931de61b1dd78cbc5878931e))
- Dashboard stat row with server status partial, polished empty states([16d84ef](https://github.com/cebarks/quartermaster/commit/16d84ef36b5815eed78bea54b1aa513ee3ba8a46))
- Add CSRF token validation on all POST forms([46640a6](https://github.com/cebarks/quartermaster/commit/46640a66deeb97ba32e9cad58693efd60d659cee))
- Add structured logging with tracing([7dddf49](https://github.com/cebarks/quartermaster/commit/7dddf49621e28efc1d24b6d6fe78c1eb5ef73db0))
- Warn about Fika incompatibility during web mod install([2efe2e1](https://github.com/cebarks/quartermaster/commit/2efe2e1d4187c846946adf4e393fd5351edf6829))
- Async mod install/update with HTMX progress tracking([6e801c9](https://github.com/cebarks/quartermaster/commit/6e801c92bea4a8971c4c599f415d49d7a630bf96))
- Load status page sections in parallel via HTMX([734ee82](https://github.com/cebarks/quartermaster/commit/734ee82b5c27e679d4e56a37983da3b99b60f44b))
- Track runtime-generated mod files separately from archive files([e746995](https://github.com/cebarks/quartermaster/commit/e74699527b516dd0eb7e48b3a3cdb5816a33bb60))
- Show runtime files in separate section on mod detail page([3fd9dc7](https://github.com/cebarks/quartermaster/commit/3fd9dc796dce43adf09bee1a892d029a662f523f))
- Add LoggingConfig structs with defaults, env var overrides, and skip_serializing_if([b9efc9a](https://github.com/cebarks/quartermaster/commit/b9efc9af2c19124c4287b0bb03e30bcff70b32b7))
- Add CLI verbosity flags and log level resolution with priority chain([eaa84ef](https://github.com/cebarks/quartermaster/commit/eaa84ef9214c18fe70fbc7df1b531be79ea85924))
- Broadcast layer, ring buffer, and reload-capable subscriber initialization([44806a1](https://github.com/cebarks/quartermaster/commit/44806a18a61569e0c8f6075729b21b5bb47b9c2f))
- Add structured logging to podman, ops, config, and CLI modules([f8026a2](https://github.com/cebarks/quartermaster/commit/f8026a2ef5ff67ee5b9b340b3ba58792a970bd45))
- Add log API endpoints (JSON + SSE) for app and server logs([b671766](https://github.com/cebarks/quartermaster/commit/b671766c864416bdcfdd8003b9ac815da7657498))
- Add web UI log viewer with tabs, level filter, search, and live tail([3ccf65f](https://github.com/cebarks/quartermaster/commit/3ccf65f084ebb2ba0e48d7933ccfd59599677dc9))
- Add human-readable file size filter and apply to mod detail page([f9608ba](https://github.com/cebarks/quartermaster/commit/f9608baebcf61e32e315c22f7f30a2b2f8e56def))
- Add size column and total footer to installed mods list([4baaae8](https://github.com/cebarks/quartermaster/commit/4baaae8aa0446faccdb430f23444cd3669ab4628))
- Move action buttons above files table and add Forge link([1416add](https://github.com/cebarks/quartermaster/commit/1416adda9407c8498f8c4c768a48a5a97471e09a))
- Add update check interval config and server-side update cache([f19c7eb](https://github.com/cebarks/quartermaster/commit/f19c7eb86504b0a76b00ef687ac565d818bd8c0f))
- Add SSE infrastructure and replace task status polling([5a96ff8](https://github.com/cebarks/quartermaster/commit/5a96ff85bd6798677fa2aebd291f9543b9b12858))
- Add cached update-status endpoint with version badges and disabled buttons([76b337e](https://github.com/cebarks/quartermaster/commit/76b337ed327dee040fe9bdc7eff9e76912d103a6))
- Add SSE-driven mod list auto-refresh and cache invalidation([abdc14f](https://github.com/cebarks/quartermaster/commit/abdc14f4ffbbeeeacc3cb8e9409a5a812bf23f78))
- Show SPT version and Tarkov version on dashboard and mods page([ff29cd4](https://github.com/cebarks/quartermaster/commit/ff29cd499ea0a78b6fd6cb2d54e74b1145adf51f))
- Add pull_image and create_spt_container to PodmanClient([3a8305d](https://github.com/cebarks/quartermaster/commit/3a8305d8bbebe5e9977862333a379cf22ba20db4))
- Add container creation to quma setup flow([d4275f4](https://github.com/cebarks/quartermaster/commit/d4275f434f18703a060955b2fbebe3b8bb96f61e))
- Add quma server create subcommand([b69bc0c](https://github.com/cebarks/quartermaster/commit/b69bc0ca62e9ad8dc1b45411d18ad6c71ba7ff3a))
- Add `just serve` recipe for launching the web UI([c323308](https://github.com/cebarks/quartermaster/commit/c323308c3130a7615d9b048e0666f165effba724))
- Add auto_start_server config option (default true)([72419eb](https://github.com/cebarks/quartermaster/commit/72419eb1ba2561f83c7fd8924d5b8788d8e9ce03))
- Auto-start server container on quma serve([a1ace65](https://github.com/cebarks/quartermaster/commit/a1ace65e9693f0af5a2542930db1d3ba23b3e7e3))
- Auto-start server container on quma serve([65b4243](https://github.com/cebarks/quartermaster/commit/65b424394e40e532732ac32c6eb903f24bc4cad3))
- *(db)* Add Role enum, disabled column, and get_user_by_id([caa1e5a](https://github.com/cebarks/quartermaster/commit/caa1e5ad540b65c75149071270e62fd0a659b773))
- Use Role enum in setup and registration, check disabled on login([d0168fc](https://github.com/cebarks/quartermaster/commit/d0168fc9515b187ae7f1a21f01dc21491d41be6b))
- DB-validated sessions with Role-based auth middleware([4394aae](https://github.com/cebarks/quartermaster/commit/4394aaeb07f389ce5a4fa41bfb4131fbed38e66f))
- Switch all handlers to capability-based auth([90f4d23](https://github.com/cebarks/quartermaster/commit/90f4d238e57249c2b1b9e8c5f1241e2080ee6341))
- Role-conditional template rendering for three-role system([4582e93](https://github.com/cebarks/quartermaster/commit/4582e939257a366e2e16718b532e1f5b28e46bd2))
- Permission model & session overhaul (multi-user sub-project 1)([09dca58](https://github.com/cebarks/quartermaster/commit/09dca58ef7fb882ad02806a4a2cf0568b1fa3ec2))
- Add DB layer for admin user management and shared invite module([541ae4a](https://github.com/cebarks/quartermaster/commit/541ae4a7e1016bd35d687326545f6c5b1d7557bc))
- Add SPT profile stats parsing for admin user panel([c1caa3d](https://github.com/cebarks/quartermaster/commit/c1caa3d60422caad353fb3f0caad392d02a61eb5))
- Session invalidation after password reset([b06b196](https://github.com/cebarks/quartermaster/commit/b06b196d555e0bed35b26cc8066e1b136956858d))
- Add admin panel templates and navigation([85b6ca4](https://github.com/cebarks/quartermaster/commit/85b6ca4c774e4d654a6d7c676d6f4833192f878d))
- Add admin panel handlers and password reset flow([3d6bc75](https://github.com/cebarks/quartermaster/commit/3d6bc755fac34722adb114488abb7b32f50dae07))
- Admin user management UI (sub-project 2)([d302f44](https://github.com/cebarks/quartermaster/commit/d302f448e496b5d6e083a02656e347e321560a85))
- Add ContainerManager backed by bollard for native Podman API([fdca155](https://github.com/cebarks/quartermaster/commit/fdca155160115f758de1c226c9fe5f1687703129))
- Add ClientsConfig for declarative dedicated client management([db51518](https://github.com/cebarks/quartermaster/commit/db515188c6bc85882846d0b95e74affb2ea8fdbe))
- Add Fika headless API client for dedicated client status([394bcad](https://github.com/cebarks/quartermaster/commit/394bcad85a226e6039f915b0419851562a60416a))
- Add ClientSupervisor with health monitoring and auto-restart backoff([bc83e0f](https://github.com/cebarks/quartermaster/commit/bc83e0fce0abeaedc84b5b6688505e97678ac7a1))
- Add convergence engine for declarative client scaling([fdd6b5c](https://github.com/cebarks/quartermaster/commit/fdd6b5cd064256f1c05138f2684c3faf0cf27358))
- Add quma client CLI commands for dedicated client management([d32d8e7](https://github.com/cebarks/quartermaster/commit/d32d8e771935fb484f83ea1de9bc762a2345a778))
- Wire ClientSupervisor into quma serve startup with convergence([618e9ab](https://github.com/cebarks/quartermaster/commit/618e9ab3f4eb5fbb3e55ef81381b442f929d5d78))
- Add /clients web UI with overview table, detail view, and dashboard widget([ee8b096](https://github.com/cebarks/quartermaster/commit/ee8b0963e54fb7a74c9b3bfb634d6f574af36776))
- Fika dedicated client management([aa610ab](https://github.com/cebarks/quartermaster/commit/aa610ab6aa8901eb1435c1cc50e51e196520d57c))
- Mod request & voting database layer (migration 007)([41097d3](https://github.com/cebarks/quartermaster/commit/41097d3c8f7b5e0b31918ab328c213123276f47b))
- Add forge_cache_ttl config option (default 24h)([5a06a9b](https://github.com/cebarks/quartermaster/commit/5a06a9bfae00c7f952df4789140dc23233d4ecfe))
- Mod request web handlers — search, create, vote, resolve([f144e08](https://github.com/cebarks/quartermaster/commit/f144e0829e98d0f72965b9feb76c088c22b5540b))
- Mod request & voting UI templates with HTMX tabs([a6c9360](https://github.com/cebarks/quartermaster/commit/a6c9360289181e770309c9c72111f19ac5ec0cc4))
- CSS styles for mod request cards and voting UI([55de110](https://github.com/cebarks/quartermaster/commit/55de110c42ea7c596b91d52bf4314a2b2e5157e8))
- Mod Request & Voting System (sub-project 3)([acda64a](https://github.com/cebarks/quartermaster/commit/acda64a7da8522b9d26721f1861cc4aaedf1bcb9))

### Miscellaneous

- Suppress clippy too_many_arguments on install_mod_from_archive([96a3724](https://github.com/cebarks/quartermaster/commit/96a3724032d4e147dc4f92b37b12d09e36e2c472))
- Global CSS polish — cards, badges, buttons, typography, new component classes([c4c41df](https://github.com/cebarks/quartermaster/commit/c4c41df9216f73e1399f57799d190ccf9a79fd7c))
- Add icons to nav links and logout button([65dc3c1](https://github.com/cebarks/quartermaster/commit/65dc3c1a70936cca7b52282881c05ae0ecc5daf3))
- Status page hero card with glow dots, 2-column grid, confirm dialogs([76af5ab](https://github.com/cebarks/quartermaster/commit/76af5abb1cba6068728093e4ec0d38fa0787d2d9))
- Mods + queue pages — icons, confirm dialogs, empty states([954b27f](https://github.com/cebarks/quartermaster/commit/954b27fa556afcf7ae02ae30a6dd0aa3cd2fb150))
- Track CLAUDE.md despite global gitignore([f1d9668](https://github.com/cebarks/quartermaster/commit/f1d9668cae74d478b1b22281b4c6f29ee10bc710))
- Add stash value TODO for admin profile cards([ae3da54](https://github.com/cebarks/quartermaster/commit/ae3da5453b073166403ad229a6a8fa75040ef2eb))
- Polish dedicated client feature — tests, clippy, scale endpoint([75eb1fc](https://github.com/cebarks/quartermaster/commit/75eb1fc106631b826d7755a8ecc7515e8b0a475a))
- Add audit and release-dry-run justfile targets([b69004c](https://github.com/cebarks/quartermaster/commit/b69004c2a7160d5062ba0f1d52c3c5e30d69bf35))
- Rename crate to spt-quartermaster([ec0f7f2](https://github.com/cebarks/quartermaster/commit/ec0f7f2f2f72ae63def04aff728c499dc26ed1e8))
- Add crate description and AGPL-3.0 license for crates.io([4e3cc5b](https://github.com/cebarks/quartermaster/commit/4e3cc5bfabf5e781a73ee6af6267664416fad731))
- Migration verification test and remove mod requests from TODO([94d7a14](https://github.com/cebarks/quartermaster/commit/94d7a148fb0e4af5ee2e00afdd06352712389c48))
- Remove accidental .venv symlink([ba240fa](https://github.com/cebarks/quartermaster/commit/ba240fa356ca0f009244ce791de5d641f9d4cb35))
- Update all dependencies to latest versions([a2c82d0](https://github.com/cebarks/quartermaster/commit/a2c82d0d36368314e6a2dbe26d0cb66e8f5ec2ad))
- Resolve Cargo.lock merge conflict([f34c031](https://github.com/cebarks/quartermaster/commit/f34c031952f594fc25bd1ca21103af92dd8a7de0))

### Performance

- Add index on installed_files.mod_id, replace N+1 query with JOIN([d076302](https://github.com/cebarks/quartermaster/commit/d0763026c6ee978cae32d5587e9311017c90194e))

### Refactoring

- Extract shared mod ops (install/update/remove) into src/ops.rs([5823b3d](https://github.com/cebarks/quartermaster/commit/5823b3ddce1c38f0c082e516a82156f4fd3f0103))
- CLI install/update/remove now delegate to shared ops([e1ab8e6](https://github.com/cebarks/quartermaster/commit/e1ab8e64772cdf66e777cc57f1b9d3288394afe4))
- Web handlers delegate to shared ops, fixes update-without-staging bug([ac122a0](https://github.com/cebarks/quartermaster/commit/ac122a0216f7e638c58c79d2792e7aa9d27d68e3))
- Web queue drain uses shared ops with dep resolution and reverse-dep cleanup([a1b79b1](https://github.com/cebarks/quartermaster/commit/a1b79b1524f6797e1ed072170efb6c6c28dbe3a5))
- Unify unmanaged-dir grouping logic between health and common([430a4f6](https://github.com/cebarks/quartermaster/commit/430a4f66cb718a4358f61d81a71e67de5a209ea6))
- Remove unused config_path from CliContext and AppState([2246377](https://github.com/cebarks/quartermaster/commit/2246377609b85cd3a42649cd994c6c96e9993568))
- Migrate all container ops from PodmanClient to bollard-backed ContainerManager([f99a648](https://github.com/cebarks/quartermaster/commit/f99a648f9babbe484cb602bdcdd543e03cd651e8))

### Testing

- Add integration tests for broadcast layer capturing tracing events([2d3957b](https://github.com/cebarks/quartermaster/commit/2d3957bb97db7c2417466a482126a2495a74d71b))
- Add password_changed_at and count_admins edge case tests([224f98c](https://github.com/cebarks/quartermaster/commit/224f98c732245e4767176806239ef63d1caf67fe))
- Add zero-raids edge case test for profile stats([6b0ee2c](https://github.com/cebarks/quartermaster/commit/6b0ee2c43f3fb5b5d9a8cd52bd2caef7e84b7f44))
- Unit tests for mod request handler helpers([5a8a4fc](https://github.com/cebarks/quartermaster/commit/5a8a4fc9f612793d47d9db1755e5dcd94713f4c5))

### Build

- Add release profiles and cargo-dist metadata([0add670](https://github.com/cebarks/quartermaster/commit/0add670b8ff24c97d2ff834e74afbdc2ed520b27))

### Security

- Fix XSS in log viewer by using textContent instead of innerHTML([5069595](https://github.com/cebarks/quartermaster/commit/50695950718066a56026032f8d06dd21f889591e))
<!-- generated by git-cliff -->
