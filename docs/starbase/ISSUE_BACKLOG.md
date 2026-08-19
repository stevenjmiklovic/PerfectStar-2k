# StarBase pre-development backlog

This backlog converts the StarBase implementation specification into GitHub-ready epics. Create these issues in the listed order so dependencies can be linked by issue number.

## MVP phases

### SB-001: Establish StarBase workspace, crate boundaries, and CI

**Labels:** `starbase`, `epic`, `phase-0`, `architecture`

Create the Rust workspace and empty crates for `starbase-core`, `starbase-store`, `starbase-mcp`, `starbase-client`, and `starbase-testkit` without changing existing PerfectStar behavior.

**Acceptance criteria**

- [ ] Existing `pstar` build and tests remain green.
- [ ] Each StarBase crate has a documented responsibility and dependency direction.
- [ ] Linux, macOS, and Windows CI jobs build the workspace.
- [ ] Stable Rust and the minimum supported Rust version are documented.
- [ ] No StarBase code performs blocking I/O on PerfectStar's TUI thread.

---

### SB-002: Record StarBase architecture decisions and threat model

**Labels:** `starbase`, `epic`, `phase-0`, `architecture`, `security`

Create ADRs for the process boundary, project-root model, SQLite/FTS design, MCP contract, and unsaved-buffer overlay extension. Add an initial threat model covering filesystem escape, local privilege inheritance, hostile manuscript content, logging, metadata writes, and process failure.

**Depends on:** SB-001

**Acceptance criteria**

- [ ] Five ADRs are reviewed and accepted.
- [ ] Architectural invariants are explicit and testable.
- [ ] Root-jail, network-deny, and manuscript-write boundaries are documented.
- [ ] MCP protocol baseline and compatibility policy are documented.
- [ ] Open security questions have owners and resolution gates.

---

### SB-010: Implement project discovery and `starbase.toml`

**Labels:** `starbase`, `epic`, `phase-1`, `project-model`

Resolve the StarBase project root from an explicit flag, nearest manifest, current document directory, or current working directory. Parse include, exclude, manuscript, notes, index, search, and privacy settings.

**Depends on:** SB-001, SB-002

**Acceptance criteria**

- [ ] Canonical root resolution is deterministic across supported platforms.
- [ ] Symlink and traversal escapes are rejected.
- [ ] Defaults exclude `.git`, `target`, `.bak`, and `.tmp~` files.
- [ ] Unsaved files do not silently create projects.
- [ ] Manifest validation produces actionable diagnostics.

---

### SB-020: Build the SQLite store, migrations, and recovery foundation

**Labels:** `starbase`, `epic`, `phase-1`, `storage`

Implement SQLite WAL storage, migrations, transactions, project identity, document metadata, index revisions, recovery snapshots, and corruption detection.

**Depends on:** SB-001, SB-002

**Acceptance criteria**

- [ ] Migrations are ordered, transactional, and tested from every fixture version.
- [ ] Failed transactions do not advance the project revision.
- [ ] Corrupt databases can be quarantined and rebuilt.
- [ ] State stays under the platform user-data directory.
- [ ] Concurrent readers remain available during index updates.

---

### SB-030: Parse documents into sections, chunks, and inline notes

**Labels:** `starbase`, `epic`, `phase-1`, `indexing`

Index UTF-8 Markdown and plain text into deterministic document, heading, section, paragraph, and `..` inline-note records while preserving original coordinates.

**Depends on:** SB-010, SB-020

**Acceptance criteria**

- [ ] Markdown heading behavior matches PerfectStar fixtures.
- [ ] Byte, character, line, and column coordinates are retained.
- [ ] CRLF, BOM, Unicode, and long-line fixtures pass.
- [ ] Oversized or unsupported files are skipped with structured diagnostics.
- [ ] Reindexing identical input yields identical logical chunks.

---

### SB-040: Implement incremental FTS5 indexing and file watching

**Labels:** `starbase`, `epic`, `phase-1`, `search`, `performance`

Build safe full-text search, deterministic ranking, incremental refresh, watcher coalescing, stale-document reporting, and atomic index revisions.

**Depends on:** SB-030

**Acceptance criteria**

- [ ] Raw FTS syntax is never accepted from users.
- [ ] Incremental refresh and clean rebuild produce equivalent logical results.
- [ ] PerfectStar's temporary-file rename save pattern creates no duplicate documents.
- [ ] Search ranking fixtures are deterministic.
- [ ] Performance targets in the StarBase specification are met.

---

### SB-045: Define citations, freshness, and source-location semantics

**Labels:** `starbase`, `epic`, `phase-1`, `protocol`

Define stable document IDs, resource URIs, heading paths, source ranges, content hashes, index revisions, pending-change state, and saved-versus-overlay provenance.

**Depends on:** SB-030, SB-040

**Acceptance criteria**

- [ ] Every returned passage can be resolved to its original source.
- [ ] Coordinates jump to the same text in PerfectStar and StarBase.
- [ ] Freshness never claims an index is current while changes are pending.
- [ ] Result envelopes are versioned and snapshot-tested.
- [ ] Large source payloads require explicit bounded range requests.

---

### SB-050: Add project notes, entities, aliases, facts, and timeline events

**Labels:** `starbase`, `epic`, `phase-2`, `story-bible`

Implement structured project memory with provenance, deterministic alias mentions, metadata revisions, archive/restore, optimistic concurrency, idempotency keys, and portable export/import.

**Depends on:** SB-020, SB-045

**Acceptance criteria**

- [ ] Every fact is sourced, user-authored, inferred, or disputed.
- [ ] Revision conflicts cannot silently overwrite metadata.
- [ ] Replayed idempotency keys do not duplicate writes.
- [ ] Export/import round trips preserve logical content.
- [ ] No automatic extraction becomes canonical without approval.

---

### SB-060: Implement MCP server initialization and protocol framework

**Labels:** `starbase`, `epic`, `phase-3`, `mcp`

Create the stdio MCP server using the official Rust SDK. Negotiate tools, resources, prompts, logging, progress, cancellation, and the experimental overlay capability.

**Depends on:** SB-001, SB-002

**Acceptance criteria**

- [ ] The server initializes in MCP Inspector.
- [ ] Stdout contains protocol frames only.
- [ ] MCP `2025-11-25` schemas and JSON Schema 2020-12 are used.
- [ ] Experimental features are explicitly negotiated.
- [ ] Unsupported capabilities are not advertised.

---

### SB-061: Expose manuscript search, context, outline, stats, and index tools

**Labels:** `starbase`, `epic`, `phase-3`, `mcp`, `search`

Implement the read-oriented MVP tools with strict schemas, structured results, text fallbacks, annotations, result caps, progress, and cancellation.

**Depends on:** SB-040, SB-045, SB-060

**Acceptance criteria**

- [ ] Every tool validates inputs and successful outputs against its schema.
- [ ] Search and context results include citations and freshness.
- [ ] Cancellation safely interrupts long operations.
- [ ] Tool descriptions contain no manuscript-controlled text.
- [ ] Error responses use stable StarBase error codes.

---

### SB-062: Expose notes, entities, and timeline MCP tools

**Labels:** `starbase`, `epic`, `phase-3`, `mcp`, `story-bible`

Implement query and upsert tools for notes, entities, and timeline events with optimistic concurrency, idempotency, provenance, and accurate MCP annotations.

**Depends on:** SB-050, SB-060

**Acceptance criteria**

- [ ] No tool writes manuscript files.
- [ ] All metadata writes are transactional and revisioned.
- [ ] Conflicts return the current revision and safe remediation.
- [ ] No hard-delete tool ships in 1.0.
- [ ] Inspector tests cover successful, conflicting, replayed, and invalid calls.

---

### SB-070: Add MCP resources, prompts, subscriptions, and notifications

**Labels:** `starbase`, `epic`, `phase-3`, `mcp`

Expose bounded `starbase://` resources, resource templates, subscriptions, list-change notifications, and the `ask_manuscript`, `review_continuity`, and `prepare_scene_brief` prompts.

**Depends on:** SB-045, SB-060, SB-061, SB-062

**Acceptance criteria**

- [ ] Resource URIs are stable and documented.
- [ ] Large manuscripts cannot be returned accidentally as a generic resource.
- [ ] Subscribers receive revision and list-change notifications.
- [ ] Prompts distinguish manuscript evidence from inference.
- [ ] Prompt content treats manuscript text as untrusted quoted data.

---

### SB-075: Complete MCP error, logging, progress, and cancellation behavior

**Labels:** `starbase`, `epic`, `phase-3`, `mcp`, `reliability`

Standardize safe errors, content-free default logs, request IDs, duration metrics, progress tokens, cancellation propagation, payload budgets, and concurrency limits.

**Depends on:** SB-060

**Acceptance criteria**

- [ ] Default logs contain no excerpts, queries, notes, or entity descriptions.
- [ ] Content logging requires an explicit diagnostic flag.
- [ ] Every cancellable loop checks its cancellation token.
- [ ] Queues and concurrent operations are bounded.
- [ ] Error messages disclose no absolute paths by default.

---

### SB-080: Add the nonblocking PerfectStar MCP client and process manager

**Labels:** `starbase`, `epic`, `phase-4`, `perfectstar-integration`

Spawn and supervise `starbase serve --stdio` from a background worker with bounded command/event channels and graceful failure behavior.

**Depends on:** SB-060

**Acceptance criteria**

- [ ] No child-process or protocol I/O runs on the TUI thread.
- [ ] Killing StarBase does not lose editor state.
- [ ] PerfectStar remains fully usable when StarBase is absent.
- [ ] Reconnect occurs on the next explicit StarBase action.
- [ ] Stderr diagnostics are bounded and available to `doctor`/status UI.

---

### SB-081: Implement session-scoped unsaved-buffer overlays

**Labels:** `starbase`, `epic`, `phase-4`, `perfectstar-integration`, `mcp`

Implement the negotiated `io.github.stevenjmiklovic.starbase.overlay` extension with replace, clear, and list operations.

**Depends on:** SB-045, SB-080

**Acceptance criteria**

- [ ] Dirty buffers override saved content for that MCP session.
- [ ] Overlay text is never persisted to disk or SQLite.
- [ ] Buffer revisions are monotonic and acknowledged.
- [ ] Saving or closing a document clears the overlay.
- [ ] Two dirty panes remain independently correct.

---

### SB-090: Build native StarBase search and navigation UI in PerfectStar

**Labels:** `starbase`, `epic`, `phase-4`, `tui`, `ux`

Add `Mode::StarBase`, `^OJ` project search, result browsing, context preview, jump-to-source, open-in-other-pane, status indicators, and cancellation.

**Depends on:** SB-061, SB-080, SB-081

**Acceptance criteria**

- [ ] Entire workflow is keyboard-only.
- [ ] New commands appear in menus, palette, and help through the central keymap.
- [ ] Search reflects dirty open buffers.
- [ ] `Esc` cancels requests or closes StarBase UI predictably.
- [ ] No StarBase action exceeds the TUI-thread work budget.

---

### SB-091: Build native notes, entity, timeline, and status UI

**Labels:** `starbase`, `epic`, `phase-4`, `tui`, `story-bible`

Add `^OI`, `^ON`, `^OL`, and `^OU` for term inspection, anchored note entry, story-bible browsing/editing, timeline viewing, and index diagnostics.

**Depends on:** SB-062, SB-090

**Acceptance criteria**

- [ ] Metadata mutations require an explicit commit action.
- [ ] Revision conflicts are understandable and recoverable.
- [ ] Citations jump back to source passages.
- [ ] Status distinguishes connected, busy, overlay, warning, and offline states.
- [ ] Ordinary editing continues during server work.

---

### SB-100: Security hardening, fuzzing, and recovery audit

**Labels:** `starbase`, `epic`, `phase-5`, `security`, `quality`

Exercise root-jail enforcement, hostile inputs, parser and protocol fuzzing, database recovery, metadata replay, logs, payload limits, and manuscript prompt-injection boundaries.

**Depends on:** All core MVP epics

**Acceptance criteria**

- [ ] Traversal, symlink, device-file, and UNC escape fixtures fail closed.
- [ ] Fuzz targets run in CI or scheduled automation.
- [ ] Corruption recovery preserves portable metadata exports.
- [ ] No manuscript content is executed or treated as tool metadata.
- [ ] Security review has no unresolved critical or high-severity finding.

---

### SB-105: Establish performance benchmarks and release budgets

**Labels:** `starbase`, `epic`, `phase-5`, `performance`, `quality`

Create reproducible indexing, search, context, overlay, memory, and TUI-latency benchmarks with regression thresholds.

**Depends on:** SB-040, SB-081, SB-090

**Acceptance criteria**

- [ ] Baseline hardware and corpus are documented.
- [ ] 100k-word and 1M-word indexing targets pass.
- [ ] Warm search and context p95 targets pass.
- [ ] TUI-thread work remains under 5 ms per poll.
- [ ] Regressions over 20 percent require explicit review.

---

### SB-110: Write documentation and ship a sample novel project

**Labels:** `starbase`, `epic`, `phase-5`, `documentation`

Create installation, project setup, MCP host, PerfectStar, privacy, recovery, troubleshooting, and contributor documentation plus a distributable fixture novel.

**Depends on:** Feature complete MVP

**Acceptance criteria**

- [ ] A new user can initialize, index, search, and jump to a result from docs alone.
- [ ] Generic MCP-host setup is documented.
- [ ] Privacy and no-network guarantees are explicit.
- [ ] Recovery procedures are tested from the published instructions.
- [ ] Sample project contains entities, notes, timeline data, and continuity fixtures.

---

### SB-120: Package StarBase for cross-platform and MCP distribution

**Labels:** `starbase`, `epic`, `phase-5`, `release`

Produce release binaries, checksums, SBOM, dependency license report, `server.json`, an MCPB artifact, crates.io packaging, and GitHub release automation.

**Depends on:** SB-100, SB-110

**Acceptance criteria**

- [ ] Packaged artifacts pass MCP Inspector.
- [ ] Linux, macOS, and Windows clean-install tests pass.
- [ ] Tagged releases are reproducible.
- [ ] Checksums and SBOM are published.
- [ ] Registry metadata names the server `io.github.stevenjmiklovic/starbase`.

---

### SB-130: Conduct the StarBase 1.0 release audit

**Labels:** `starbase`, `epic`, `phase-5`, `release`

Audit all ten product-star promises, migration paths, documentation, security, protocol conformance, packaging, and disaster recovery before tagging 1.0.

**Depends on:** All MVP epics

**Acceptance criteria**

- [ ] All ten product-star gates pass.
- [ ] Every public prerelease database fixture upgrades successfully.
- [ ] No critical or high known dependency advisory remains.
- [ ] Offline sample-project acceptance test passes.
- [ ] Release is approved against the immutable 1.0 checklist.

## Post-MVP epics

### SB-200: Deterministic continuity engine

Implement evidence-backed rules for conflicting facts, aliases, introductions, chronology, elapsed time, geography, terminology, and tagged unresolved threads.

### SB-300: Optional semantic intelligence

Add opt-in semantic search and model-assisted proposals with explicit consent, provider provenance, and mandatory human confirmation.

### SB-400: Authenticated remote and team operation

Design Streamable HTTP, authorization, project scopes, multi-user revisions, audit trails, and encrypted remote storage under a new threat model.

### SB-500: StarBase ecosystem and editor-neutral integrations

Design parser plugins, export adapters, an editor-neutral overlay contract, and integrations beyond PerfectStar.
