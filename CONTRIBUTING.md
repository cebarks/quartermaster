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

### Security audits

Install Python 3 and the same Cargo Audit version used in CI:

```bash
cargo install cargo-audit --locked --version 0.22.2
just audit
```

`just audit` runs the audit-policy regression tests and `scripts/audit.py`, the same gate used in CI. To run only the policy tests without network access:

```bash
python3 -m unittest discover -s scripts -p 'test_audit.py' -v
```

There is one temporary risk acceptance: **RUSTSEC-2026-0258 for registry `h2` 0.3.27 only** (`registry+https://github.com/rust-lang/crates.io-index`). Actix still depends on that unpatched release; the [upstream migration issue](https://github.com/actix/actix-web/issues/4199) is open and the [old-series backport was declined](https://github.com/hyperium/h2/pull/948#issuecomment-5587273695).

This is a real HTTP/2 memory-exhaustion/denial-of-service risk, not an unused dependency or a patched library. Native TLS and HTTP/2 behavior remain unchanged. The gate prints the acceptance on every affected run and still fails for the same advisory on any other version or source, any other vulnerability, or an audit execution/report error. Warnings remain visible. Advisory-wide ignores, severity/target filters, and suppression of the default `unmaintained`, `unsound`, and `notice` warning categories are rejected rather than silently hiding findings.

Remove the exception from `scripts/audit.py` when Actix no longer brings in this vulnerable release. Do not add the advisory to Cargo Audit's ignore list: that would also hide affected `h2` 0.4 versions. Raw `cargo audit` intentionally remains nonzero while the accepted finding exists; `just audit` is the policy-aware local/CI check.

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
