# ADR-008: Cargo Workspace Restructure

**Date:** 2025-07-14
**Status:** Draft
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

PerfectStar 2k is currently a single Cargo package with one binary (`pstar`). The addition of `pstar-radio` (a standalone YouTube radio TUI) introduces a second binary that shares dependencies (ratatui, crossterm, serde, dirs) and the theme system with the editor. A third crate (`pstar-common`) is needed to hold shared code without circular dependencies. The repo must be restructured to support multiple crates while preserving the existing build, binary name, and development workflow.

## Decision Drivers

- `pstar-radio` needs the same theme palette and config-path helpers as `pstar` — duplication would drift
- Both binaries share heavy dependencies (ratatui, crossterm, serde_json, dirs, toml) — a workspace deduplicates them in `Cargo.lock`
- The existing `pstar` binary name, package metadata, and `cargo install` behavior must not regress
- Future workspace members (e.g., a library crate for in-process radio embedding) should be easy to add
- CI, `cargo clippy`, and `cargo test` should work at the workspace level with one command

## Considered Options

1. **Cargo workspace with three members: `pstar/`, `pstar-common/`, `pstar-radio/`**
2. **Two independent repos** — `pstar-radio` as a completely separate project with its own `Cargo.lock`
3. **Feature-flag binary** — a single crate with `--features radio` conditionally compiling the radio binary
4. **Git submodule** — `pstar-radio` repo included as a submodule in PerfectStar-2k

## Decision Outcome

**Chosen option:** Cargo workspace (option 1), because it provides shared dependency resolution, shared code via `pstar-common`, a single `Cargo.lock`, and workspace-wide commands while keeping each binary independently publishable.

### Positive Consequences

- Shared dependencies are resolved once; `Cargo.lock` stays coherent across all crates
- `pstar-common` provides a clean extraction point for shared types (themes, path helpers) without circular deps
- `cargo build --workspace` compiles everything; `cargo run -p perfectstar2k` and `cargo run -p pstar-radio` target individual binaries
- Adding future workspace members (e.g., `pstar-lib` for embedding) is a one-line change to the workspace manifest
- Each crate can have its own `Cargo.toml` metadata for independent publishing if desired

### Negative Consequences

- The restructure moves `src/` to `pstar/src/`, which is a large diff touching every file path (though no code changes)
- `cargo install perfectstar2k` must still work — the editor crate's `Cargo.toml` must keep its `[[bin]]` section and package name
- Contributors must understand workspace-relative paths for `cargo run -p <name>`

## Implementation Notes

The top-level `Cargo.toml` becomes:

```toml
[workspace]
members = ["pstar", "pstar-common", "pstar-radio"]
resolver = "3"
```

The existing `src/` moves to `pstar/src/` with the editor's `Cargo.toml` preserving its `package.name = "perfectstar2k"` and `[[bin]] name = "pstar"`.

## Links and References

- Spec: `.claude/specs/pstar-radio/requirements.md` (Task 1)
- Cargo workspaces documentation: https://doc.rust-lang.org/cargo/reference/workspaces.html
