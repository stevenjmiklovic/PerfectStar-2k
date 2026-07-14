# ADR-013: Hand-Generated DOCX and EPUB Containers

**Date:** 2026-07-14
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

Professional export requires DOCX and EPUB while preserving headings, emphasis, paragraph structure, project binder order, and an EPUB table of contents. Both formats are ZIP containers of standardized XML. The implementation must work fully offline and fail without replacing a previous good export.

The Phase 3 specification originally called this decision “ADR-009,” but that number is already occupied by the polling event-loop decision. This record uses the next available repository number.

## Decision Drivers

- Fully offline operation with no external converter
- Reproducible output suitable for golden-file testing
- Small prose-focused feature set: headings, paragraphs, emphasis, code, page breaks, and EPUB navigation
- Shared normalized document semantics across RTF, HTML, DOCX, and EPUB
- No dependency weight or transitive supply-chain cost for functionality the standards make tractable
- Atomic destination replacement after complete generation

## Considered Options

1. Hand-generate minimal XML and ZIP container records
2. Use `docx-rs` and `epub-builder`
3. Shell out to Pandoc, LibreOffice, or another converter

## Decision Outcome

**Chosen option:** hand-generate the required XML and a minimal deterministic, stored-entry ZIP container.

A shared `CompiledDoc` supplies normalized blocks and emphasis runs. DOCX emits the minimal OPC content-types, relationship, and WordprocessingML parts. EPUB emits EPUB 3 container/package metadata, XHTML content, and a navigation document generated from headings in compiled binder order. ZIP entries are uncompressed and timestamped deterministically; EPUB's `mimetype` is first and uncompressed as required.

### Positive Consequences

- DOCX and EPUB exports work with no network, system package, or converter
- Output is deterministic and straightforward to inspect in tests
- One normalized model prevents format-specific note stripping or typography drift
- Export rendering completes before atomic temp-file replacement, preserving prior output on error
- No new Cargo dependency is introduced

### Negative Consequences

- The implementation supports prose documents, not the full DOCX or EPUB feature sets
- ZIP64 and compression are intentionally unsupported; exports are limited to classic ZIP's 4 GiB bounds
- Compatibility must be maintained against office readers and EPUB validators as standards evolve
- Images, tables, footnotes, stylesheets, and embedded fonts require future explicit work

## Links and References

- Shared model and atomic write: `src/export/mod.rs`
- DOCX writer: `src/export/docx.rs`
- EPUB writer: `src/export/epub.rs`
- Deterministic ZIP writer: `src/export/zip.rs`
- Prior export decision: `docs/adr/ADR-006-hand-generated-rtf-manuscript-export.md`
