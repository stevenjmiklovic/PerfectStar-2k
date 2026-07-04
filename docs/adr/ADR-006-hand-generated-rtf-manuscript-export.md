# ADR-006: Hand-Generated RTF for Manuscript Export

**Date:** 2025-06-15
**Status:** Accepted
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

Fiction writers submitting to literary agents and editors must produce manuscripts in Standard Manuscript Format (SMF): 12pt serif, double-spaced, 1-inch margins, first-line-indented paragraphs, chapters on fresh pages. The editor's `^KM` command must export the Markdown source directly to a submission-ready RTF file. The export must handle Markdown emphasis (`*italic*`, `**bold**`), smart typography (curly quotes, em dashes, ellipses), note-line stripping, and chapter page breaks — all without introducing an external converter dependency.

## Decision Drivers

- Zero external dependencies: no pandoc, no LibreOffice, no system-installed tools
- RTF is universally accepted by literary agents and readable by every word processor
- The RTF spec is plain ASCII with brace-delimited control words — hand-generation is tractable
- Smart typography (curly quotes, em dashes) must be applied during export so the writer types plain ASCII in the editor
- `..` note lines (writer's notes to self) must be stripped, matching `^KE` behavior
- Markdown markup must be stripped and replaced with real RTF emphasis — the output should not contain literal asterisks

## Considered Options

1. **Hand-generated RTF** — emit RTF control words directly from a custom renderer
2. **Pandoc integration** — shell out to pandoc for Markdown → RTF conversion
3. **DOCX generation** — produce a `.docx` (ZIP of XML) with a library like `docx-rs`
4. **PDF export** — render to PDF via a typesetting engine

## Decision Outcome

**Chosen option:** Hand-generated RTF, because the subset of RTF needed for manuscript format is small (font table, paragraph formatting, bold/italic runs, page breaks), the output is deterministic and testable, and the editor gains no external dependency. A ~200-line renderer produces submission-ready output.

### Positive Consequences

- No external tools required — the export works on any system with just the `pstar` binary
- Complete control over the output: every paragraph reset, every `\page` break, every `\fi720` indent is explicit
- Smart typography (curly quotes, em dashes, ellipses) is applied inline during rendering without a separate pass
- Markdown markers are stripped by the same `markdown::scan_line` used for rendering — consistency between what the writer sees and what's exported
- The RTF is valid and opens correctly in Word, Pages, LibreOffice, and Google Docs
- Configurable font (`times` or `courier` via `config.toml`) affects only the RTF `\f` index

### Negative Consequences

- The renderer must be maintained manually if new Markdown constructs are added (e.g., strikethrough, footnotes)
- RTF's escaping rules (backslash, braces, non-ASCII via `\uN` codes) must be handled correctly — the `escape_rtf` function is a critical correctness surface
- No support for images, tables, or complex formatting — this is intentionally a prose-only exporter
- The smart-typography state machine (tracking open/close quotes across a line) is subtle and bug-prone at edge cases

## Links and References

- Implementation: `src/rtf.rs` (`render`, `render_line`, `escape_rtf`, `smart_char`)
- Markdown scanning: `src/markdown.rs` (same `scan_line` drives both display and export)
- Configuration: `src/config.rs` (`manuscript_font` field, `ManuscriptFont` enum)
- Branch: main
