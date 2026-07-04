# ADR-005: Bundled Hunspell Dictionary via Spellbook (Offline Spellcheck)

**Date:** 2025-06-15
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

Writers expect real-time spellcheck — misspelled words underlined as they type. The editor must work offline, on any platform, without depending on a system spell-checking service (macOS NSSpellChecker, aspell, hunspell CLI) or a network API. The dictionary must be bundled so the binary is self-contained and works immediately after `cargo install`.

## Decision Drivers

- Zero runtime dependencies on system spell services — works on a fresh Linux/macOS/Windows install
- Offline-first: no network calls, ever
- Hunspell-compatible affix support (so inflected forms like "running" from "run" resolve correctly)
- Personal dictionary for character names, invented words, and jargon — persisted globally across documents
- Minimal binary size impact relative to the dictionary data itself

## Considered Options

1. **`spellbook` crate with bundled en_US.aff/dic via `include_str!`** — pure Rust Hunspell implementation, dictionary compiled into the binary
2. **`hunspell-rs` (FFI to libhunspell)** — full-featured but requires libhunspell installed on the system
3. **`symspell` / Levenshtein-based** — fast suggestions but no affix morphology, high false-positive rate for creative writing
4. **System spellcheck APIs** — platform-specific, unavailable on Linux servers, non-portable

## Decision Outcome

**Chosen option:** `spellbook` with the SCOWL en_US dictionary bundled via `include_str!`, because it provides Hunspell-compatible affix expansion in pure Rust with zero system dependencies, and the dictionary compiles directly into the binary for true single-file distribution.

### Positive Consequences

- The editor binary is entirely self-contained — no dictionary files to install or discover at runtime
- Affix expansion means the dictionary covers inflected forms without listing every variant
- The personal dictionary (`~/.config/perfectstar2k/personal_dict.txt`) is a plain-text wordlist — trivially editable by hand, shared across all documents
- Spellcheck integrates cleanly with the rendering pipeline: `word_spans()` produces char ranges, and `Style::patch(misspelled)` layers the underline on top of existing Markdown styling without replacing it
- Acronyms (all-caps) and digit-containing tokens pass automatically — no false positives on "NASA" or "v2.0"

### Negative Consequences

- The bundled dictionary adds ~5 MB to the binary (compressed in the executable's data segment)
- Only en_US is bundled; supporting additional languages would require shipping separate dictionary files or a language-selection mechanism
- `include_str!` means the dictionary is loaded into memory at startup regardless of whether spellcheck is enabled (mitigated: the `Dictionary` struct is only constructed once)
- Suggestion generation (for a future "did you mean?" feature) would require additional work — `spellbook` focuses on checking, not suggesting

## Links and References

- Implementation: `src/spellcheck.rs` (`Spellchecker` struct, `check`, `learn`, `word_spans`)
- Dictionary assets: `assets/en_US.aff`, `assets/en_US.dic`
- Attribution: `THIRD-PARTY-NOTICES.md` (SCOWL / Kevin Atkinson, Ispell / Geoff Kuenning)
- Personal dict location: `~/.config/perfectstar2k/personal_dict.txt`
- Branch: main
