# ADR-015: Fixed Bundled Style Rules, Individually Toggleable

**Date:** 2026-08-19
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

R8 asks for style checking that flags passive voice, `-ly` adverbs, filter and crutch words, and very long sentences — rendered like spelling but distinguishable from it, individually toggleable, fully offline, and with no perceptible typing cost. The open question (design §7.4) is whether the rules are a fixed bundled set or user-extensible, because that choice decides the shape of both the engine and the config surface.

The Phase 11 specification called this decision "ADR-011," which is already the shell-out integration record. This uses the next available repository number.

## Decision Drivers

- The four required checks are specific and well-defined; nothing in R8 asks for a fifth
- Config surface should stay legible in a hand-edited `config.toml`
- Style advice is opinionated, and a writer's real need is to *silence* advice they disagree with, not to author new advice
- User-extensible rules imply a rule language — regex at minimum — with its own escaping, error reporting, and pathological-input risks inside the render path (C6)
- Fully offline, no rule downloads, no rule marketplace (C5)
- The bundled word lists are data, not code, and can become configurable later without changing the engine

## Considered Options

1. Fixed bundled rules, each toggleable, with a numeric threshold for sentence length
2. User-extensible rules defined by regex in `config.toml`
3. A small rule DSL or embedded scripting hook

## Decision Outcome

**Chosen option:** fixed bundled rules, individually toggleable.

`StyleEngine` holds a `StyleChecks` record — `passive`, `adverbs`, `filler`, `long_sentences`, plus `sentence_words` as the long-sentence threshold — read from `config.toml` (R8.7). Each rule is a Rust function over a line's word and sentence spans. The crutch/filter word list and the irregular-participle list are bundled `const` data.

This resolves the writer's actual complaint. Someone who finds the adverb check nagging turns off `style_adverbs`; someone writing a thriller who wants shorter sentences flagged sets `style_sentence_words = 20`. Nobody has asked to invent a new class of style advice, and a regex rule running against every rendered line is a latency and footgun surface (catastrophic backtracking in a draw path) with no requirement behind it.

The word lists being `const` data rather than logic leaves the extension point open: a future `style_filler_extra = [...]` config key would add to the bundled list without touching the engine's shape or this decision.

### Positive Consequences

- No rule language to design, document, escape, or validate
- Rules are ordinary Rust, so they are unit-testable per fixture and cannot backtrack pathologically
- Config surface is four booleans and one number
- Fully offline and deterministic, like the bundled dictionary (ADR-005)

### Negative Consequences

- A new check requires a release, not a config edit
- Writers with idiosyncratic crutch words (their own tics) can't add them yet; the extension point above is the answer when someone asks
- The checks are heuristic — `-ly` matching and be-plus-participle detection have no part-of-speech tagger behind them, so they carry a known false-positive rate, documented in `src/style.rs`

## Links and References

- Rule engine: `src/style.rs`
- Config keys: `src/config.rs`
- Prior bundled-data decision: `docs/adr/ADR-005-bundled-hunspell-spellcheck.md`
- Rendering pattern this mirrors: `src/spellcheck.rs`, `src/ui.rs`
