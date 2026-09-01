# Progress: PerfectStar 2k v0.5.0 Release

## Status

In progress

## Last Completed Step

Created persistent release planning files and confirmed the current package version, changelog head, branch, repository, and GitHub authentication.

## Next Action

Inspect release history, tags, changelog fragments, and repository conventions, then determine the exact v0.5.0 release contents.

## Blockers

- None currently.

## Significant Step

Release history and fragments were mapped: v0.4.0 is the only existing tag, and HEAD contains the post-0.4 feature work for snapshots, sprints, metadata/notes, style rules, lookup/autocorrect, onboarding hints, and performance harnesses. `gh` is authenticated and no GitHub releases exist yet. Validation found the dirty manifest is broken because runtime dependencies are absent.

## Significant Step

Updated package/changelog versions to 0.5.0, added release notes dated 2026-09-01, restored runtime dependencies, and corrected the GitHub URL. Fixed incremental statistics cache invalidation for newline-changing edits. Validation: 384 tests pass, clippy passes with existing warnings, and release build succeeds.
