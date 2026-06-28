# SPT Remote Client Connectivity

Investigation into why SPT game clients fail to connect to a remote dedicated server through Quartermaster's proxy.

## Root Cause: Hardcoded Backend URLs

SPT 4.x hardcodes all backend URLs to `https://127.0.0.1:6969` regardless of the `backendIp`/`backendPort` values in `SPT_Data/configs/http.json` or the Fika server config override. This affects three endpoints:

| Endpoint | Fields | Purpose |
|---|---|---|
| `/launcher/server/connect` | `backendUrl` | Primary backend URL sent to launcher/game |
| `/client/game/config` | `backend.{Lobby,Trading,Messaging,Main,RagFair}` | Per-subsystem API URLs |
| `/client/notifier/channel/create` | `ws`, `notifierServer`, `server` | WebSocket notifier connection |

The SPT Docker container's entrypoint (`LISTEN_ALL_NETWORKS=true`) also force-resets `backendIp` to `0.0.0.0` on every boot, overwriting any manual config changes.

## The SPT Launcher URL Pass-Through

The SPT launcher reads the server URL from `SPT/user/launcher/config.json` and passes it to the game process. Stock SPT always passes `https://127.0.0.1:6969` regardless of the config — the **Fika Installer** patches `Assembly-CSharp.dll` (via SPT's delta patches in `SPT_Data/Launcher/Patches/`) to make the game use the launcher config URL instead.

On Linux, the Fika Installer CLI (`wine Fika-Installer.exe install fika`) crashes during shortcut generation (COM interop) but successfully applies the delta patches before that. The Fika client plugin must be installed manually afterward.

## WebSocket Incompatibility with L7 Proxies

SPT's notifier WebSocket (`/notifierServer/getwebsocket/{sessionId}`) breaks under any L7 HTTP reverse proxy (Caddy, nginx http, npm, Quartermaster's actix-web proxy). The game hangs at "loading profile" and eventually shows "Backend error: Cannot connect to destination host". This was independently confirmed by other users in the Fika community.

Fika also creates a separate WebSocket at `/fika/notification/` that exhibits the same behavior.

The WebSocket handshake (101 Switching Protocols) succeeds, but the ongoing connection framing breaks under L7 proxying.

## Solution: Split URL Routing

Quartermaster's proxy rewrites the hardcoded `127.0.0.1:6969` URLs in the three affected endpoints using different targets:

- **HTTP API endpoints** (`/launcher/server/connect`, `/client/game/config`): Rewrite to `external_url` hostname (e.g., `tarkov.grovest.io`) for L7 proxying through Caddy on port 443. This preserves Quartermaster proxy features (raid tracking, metrics).
- **WebSocket/notifier endpoint** (`/client/notifier/channel/create`): Rewrite to `hostname:6969` for direct TCP passthrough, bypassing L7 proxies entirely.

The TCP passthrough on port 6969 is handled via iptables DNAT on the reverse proxy machine:

```bash
sudo iptables -t nat -A PREROUTING -p tcp --dport 6969 -j DNAT --to-destination <server_ip>:6969
sudo iptables -t nat -A POSTROUTING -p tcp -d <server_ip> --dport 6969 -j MASQUERADE
sudo ufw route allow proto tcp to <server_ip> port 6969
```

The response body may be zlib-compressed (SPT default) or plain JSON (if the client sends `responsecompressed: 0`). The rewrite handles both formats.

## Alternative: Full Direct Connection

If L7 proxy features aren't needed, clients can connect directly to the SPT server on port 6969 (e.g., `https://hostname:6969` in the launcher config). This bypasses all proxies and avoids the WebSocket issue entirely, but loses Quartermaster's proxy-based features.

## Client Setup Requirements

For a remote Fika server on Linux:

1. Install SPT client (via SPT installer or spt-additions script)
2. Run the Fika Installer CLI to patch `Assembly-CSharp.dll`: `wine Fika-Installer.exe install fika`
3. Install the Fika client plugin (`BepInEx/plugins/Fika/Fika.Core.dll`) manually
4. Set the server URL in `SPT/user/launcher/config.json`
5. Sync client-side mods from the server (all server mods with client components must be present)

## Gotchas

- The Fika Installer's `uninstall` command removes the Fika client plugin but does NOT revert the `Assembly-CSharp.dll` patch. A fresh `install fika` will re-apply patches but crash on shortcut generation (Wine COM issue) before downloading plugins.
- `DynamicMaps` requires `Unity.VectorGraphics.dll` in `EscapeFromTarkov_Data/Managed/` — missing this causes a hard crash at startup with a `ReflectionTypeLoadException`.
- Server-side mods that add custom item templates (e.g., WTT-PackNStrap's `CustomContainerTemplate`) will cause client errors if the corresponding client mod isn't installed or can't load due to missing dependencies.
- NarcoNet (client-side modsync) expects NarcoNet server endpoints (`/narconet/version`), which don't exist when Quartermaster provides modsync instead.
