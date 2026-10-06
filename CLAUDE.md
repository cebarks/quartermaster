# CLAUDE.md

@AGENTS.md

This file is an import shim, not a second source of truth. Project instructions live in
`AGENTS.md`, the tool-neutral standard (https://agents.md/) that Pi, Codex CLI, GitHub
Copilot, Cursor, Windsurf and Claude Code all read.

Claude Code reads a `CLAUDE.md` in preference to `AGENTS.md` and ignores the latter
entirely when one exists, so the `@AGENTS.md` import above is how this repository's
guidance reaches a Claude session. The import is expanded at launch and is never loaded
twice, whichever **Project instructions** value is configured.

- Put Claude-specific additions *below* the import line; anything above it is imported first.
- Never copy `AGENTS.md` content here — a duplicated rule here would shadow the shared file and drift out of date.
- If this file ever holds nothing but the heading and the import, it can still be deleted: Claude Code v2.1.277+ falls back to `AGENTS.md` directly. Keep the shim for older versions and for sessions where `AGENTS.md` cannot load.
