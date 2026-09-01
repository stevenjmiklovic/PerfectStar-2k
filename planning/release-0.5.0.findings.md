# Findings: PerfectStar 2k v0.5.0 Release

## Key Decisions

- The requested target version is `0.5.0`, a backward-compatible release bump from `0.4.0`, so the package version and changelog need a MINOR SemVer update.
- GitHub CLI authentication is available for `stevenjmiklovic`; the repository is `stevenjmiklovic/PerfectStar-2k`.

## Dead Ends (Do Not Retry)

- The installed whetstone power does not expose a `release-manager` steering file in its available steering list; use the documented companion-workflow description and the repository's own release conventions instead of retrying unavailable steering lookup.

## Research Notes

- Current branch: `feat--pro-writing`, tracking `origin/feat--pro-writing`; working tree has pre-existing untracked `planning/` content.
- `Cargo.toml` currently declares version `0.4.0`.
- `CHANGELOG.md` currently begins with release `0.4.0` dated 2026-07-14.
- GitHub CLI is installed and authenticated with repository permissions.
- Release validation exposed a broken dirty-tree manifest: the current `Cargo.toml` retained only `proptest` as a dev dependency while all runtime dependencies were removed, causing `cargo test` to fail with 112 unresolved-import errors. The property-test additions in `src/meta.rs`, `src/snapshot.rs`, and `src/sprint.rs` appear intentional and will be preserved; the runtime dependency declarations must be restored before validation.
- Release metadata now targets `stevenjmiklovic/PerfectStar-2k`; the stale `exarcos` URL in `changelogging.toml` was corrected while preserving the existing fragment-based changelog setup.
- After restoring runtime dependencies and fixing line-count cache invalidation, `cargo test` passes all 384 tests and `cargo build --release` succeeds.
- `cargo clippy --all-targets --all-features` succeeds with 19 existing warnings. `-D warnings` is not release-clean because of those pre-existing dead-code and style warnings across the codebase; no broad warning cleanup was added to this release-preparation change.
- `cargo fmt` was run before validation. Release metadata and property-test changes are tracked; unrelated pre-existing untracked planning files remain outside the staged release set.