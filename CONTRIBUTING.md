# Contributing to Quartermaster

## Setup

1. Install Rust (2021 edition) and [just](https://just.systems/)
2. Clone the repo and bootstrap a local dev environment:

```bash
just dev-init       # sets up a real SPT dev environment at .dev-server/
just dev-serve      # build and run the web UI against it
```

3. Install git hooks for pre-commit linting:

```bash
just install-hooks
```

## Development Workflow

**Always work in a branch** — don't commit directly to main. Use descriptive branch names like `feature/health-regen` or `fix/collision-bug`.

```bash
just build          # cargo build
just test           # cargo test
just lint           # fmt + clippy + logging conventions + copy-paste detection
just dev-watch      # auto-rebuild on file changes (needs cargo-watch)
```

Run `just dev-seed` to populate the dev database with test data, or `just dev-reset-db` to wipe it.

### Worktree-aware dev environment

The `dev-*` recipes auto-detect git worktrees and derive unique ports and container names, so multiple branches can run in parallel without conflicts. Run `just dev-info` to see the current settings.

## Testing

```bash
just test                       # run all tests
cargo test <test_name>          # run a specific test
```

Write tests for new features and bug fixes. Integration tests often provide more value than unit tests.

## Code Style

- `just lint` runs formatting, clippy, logging convention checks, and copy-paste detection — all must pass
- Keep functions focused; avoid premature abstraction
- Only comment code that is confusing or non-obvious

## Forge API

If you're working on the Forge client (`src/forge/`), see [docs/forge-api-notes.md](docs/forge-api-notes.md) for undocumented API quirks and behaviors.

## Commits

Group related changes into logical, complete commits. Write clear messages that explain *why*, not just *what*.

## AI Disclosure

This project uses LLM-based tools (Claude Code) for implementation assistance. All architecture, design, and technical direction are human-driven. See the [AI Disclosure](README.md#ai-disclosure) section in the README.
