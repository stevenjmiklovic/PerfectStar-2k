# ADR-016: Bundled Offline Lookup Resource with Typed Unavailable State

**Date:** 2026-08-19
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

R10 asks for a thesaurus and definition lookup for the word under the cursor, presented in a dismissable overlay, working offline (C5), returning results within 500ms (R10.1, R10.2). The open question is *where the lexical data comes from*: a bundled resource compiled into the binary, a resource read from the platform data directory at runtime, or a network dictionary/thesaurus service.

R10.7 additionally requires that when a lookup resource is not bundled or fails to load, the system posts a non-blocking message and disables the corresponding command — rather than producing an unhandled error. That is a statement about the *shape of the loader's return type*, not just runtime behavior, and deserves to be recorded alongside the sourcing decision because the two choices reinforce each other.

## Decision Drivers

- **C5 — offline and dependency-light.** Core writing features must work with no network and no external binaries, consistent with bundled spellcheck (ADR-005) and hand-generated RTF (ADR-006).
- **C6 — performance.** Lookup must return within 500ms; an in-memory map built from compiled-in data is trivially within budget, a network round-trip is not and is not even available offline.
- **R10.7 degradation.** A missing or unloadable resource must never panic or surface a raw error; it must be a first-class, typed state the command layer can branch on.
- **C4 — plain files.** Any on-disk resource should be a plain, inspectable file, not an opaque blob.
- Precedent: ADR-005 already bundles the Hunspell dictionary via `include_str!` and treats the dictionary as offline data compiled into the binary.

## Considered Options

1. **Bundled resource compiled in via `include_str!`**, parsed into in-memory maps at startup, with a typed `LookupState::{Ready, Unavailable}` loader.
2. **Runtime resource file** read from the platform data dir (like sessions/snapshots), downloaded or installed separately.
3. **Network lookup service** (WordNet API, dictionary API, etc.) queried per invocation.

## Decision Outcome

**Chosen option:** a bundled offline resource compiled into the binary, with a typed unavailable state.

`lookup.rs` owns a `LookupResource` parsed from a bundled plain-text data file (`assets/thesaurus.txt`) via `include_str!`, exactly as `spellcheck.rs` bundles `en_US.aff`/`en_US.dic`. Parsing produces two in-memory maps — synonyms and definitions — keyed by lowercased head-word, so a lookup is an O(1) map hit well inside the 500ms budget (R10.1) and works with no network (C5).

The loader never panics. Loading is expressed as:

```rust
enum LookupState {
    Ready(LookupResource),
    Unavailable(Unavailable), // carries which resource + a human reason
}
```

The bundled path (`LookupResource::bundled()`) is effectively always `Ready` because the data is compiled in. But the parser is also reachable via `LookupResource::from_str`, which returns the same `LookupState`, so an empty or malformed resource yields `Unavailable` rather than a half-built map or a panic. This gives the command layer (a later task) a single value to branch on to satisfy R10.7: post a non-blocking status message naming the unavailable resource and disable the `Thesaurus`/`Define` commands until it is available.

Option 3 fails C5 outright (no offline operation) and C6 (network latency). Option 2 adds an install/download step and a first-run "resource missing" state that contradicts the "works out of the box, offline" promise the bundled spellcheck already sets; it also multiplies the failure surface. Bundling keeps lookup as reliable and dependency-free as spellcheck.

The resource being a plain bundled file rather than logic leaves the extension point open: a future config key could point at a larger user-supplied resource, loaded through the same `from_str`/typed-state path, without changing the engine's shape or this decision.

### Positive Consequences

- Works fully offline and deterministically, like the bundled dictionary (ADR-005).
- Lookups are in-memory map hits, trivially within the R10.1/R10.2 latency budget.
- The typed `LookupState` makes R10.7 a compile-time-checked branch, not an afterthought — no unhandled error path.
- No network, no external tool, no per-invocation I/O.

### Negative Consequences

- The bundled resource is necessarily compact (binary-size trade-off), so coverage is smaller than a full WordNet; the extension point above is the answer when a writer wants more.
- Growing the resource requires a release, not a config edit — acceptable for the same reasons as the bundled dictionary and style lists (ADR-005, ADR-015).

## Links and References

- Lookup module: `src/lookup.rs`
- Bundled resource: `assets/thesaurus.txt`
- Prior bundled-data decisions: `docs/adr/ADR-005-bundled-hunspell-spellcheck.md`, `docs/adr/ADR-015-fixed-bundled-style-rules.md`
- Prose normalization reused by R10 typography: `src/normalize.rs`
