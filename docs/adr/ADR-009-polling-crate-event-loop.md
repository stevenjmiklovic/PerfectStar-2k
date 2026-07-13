# ADR-009: Polling Crate for Lightweight Event Loop

**Date:** 2025-07-14
**Status:** Draft
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

`pstar-radio` must multiplex three I/O sources: terminal keyboard input (stdin via crossterm), mpv's IPC socket (JSON-RPC messages), and timer ticks (marquee animation, elapsed-time polling). The existing `pstar` editor uses a simple synchronous loop (`crossterm::event::poll(100ms)` + `event::read()`), which suffices for a single input source. The radio needs to react to mpv events without blocking on terminal input, and vice versa.

## Decision Drivers

- Consistency with pstar's synchronous, single-threaded architecture — no `async fn main()`, no `.await`
- Must watch at least two file descriptors (stdin + mpv socket) simultaneously
- Must support both Unix (poll/epoll/kqueue) and Windows (IOCP for named pipes) platforms
- Minimal dependency footprint — the radio is a small focused binary
- No need for task scheduling, futures, or multi-threaded execution

## Considered Options

1. **`polling` crate** — a thin, cross-platform reactor wrapping epoll/kqueue/IOCP, exposing `Poller::wait()` with timeout
2. **Tokio** — full-featured async runtime with task spawning, timers, I/O drivers
3. **`mio`** — lower-level cross-platform I/O event notification (the foundation under Tokio)
4. **Stay fully synchronous** — alternate between `crossterm::event::poll` and non-blocking socket reads with manual timeout logic

## Decision Outcome

**Chosen option:** The `polling` crate (option 1), because it provides exactly the "watch multiple fds with a timeout" primitive needed, without pulling in an async runtime, futures, or a task scheduler. It is a minimal, well-maintained crate that wraps platform-specific APIs behind a single `Poller` interface.

### Positive Consequences

- The event loop stays synchronous and single-threaded — easy to reason about, consistent with pstar
- `Poller::wait(&mut events, Some(Duration::from_millis(100)))` replaces the current `crossterm::event::poll` pattern naturally
- Cross-platform by design: uses epoll on Linux, kqueue on macOS, IOCP on Windows
- Tiny dependency tree compared to Tokio (~5 transitive deps vs ~50+)
- Timer ticks fall out naturally from the poll timeout — no separate timer infrastructure needed

### Negative Consequences

- Slightly more manual than Tokio for handling buffered reads (must manage partial-line buffering on the mpv socket ourselves)
- If future features need true concurrency (e.g., parallel channel fetches), would need to add threads or reconsider — but this is unlikely for a radio player
- `polling` operates at the fd level; crossterm's `event::poll` is a higher-level abstraction that may need to coexist (crossterm's internal poll can be bypassed by reading its fd directly on Unix, or by using `crossterm::event::poll` with zero timeout after `polling` signals stdin readiness)

## Links and References

- Spec: `.claude/specs/pstar-radio/requirements.md` (Section 3.3, Task 6)
- `polling` crate: https://crates.io/crates/polling
- ADR-008: Cargo workspace restructure (provides the workspace context)
