# ADR-010: Cross-Platform mpv IPC via Trait Abstraction

**Date:** 2025-07-14
**Status:** Draft 
**Deciders:** Steven Miklovic
**Supersedes:** N/A

## Context and Problem Statement

`pstar-radio` controls mpv (the audio backend) through mpv's JSON-based IPC protocol. On Unix systems (macOS, Linux), mpv exposes this via a Unix domain socket. On Windows, mpv uses a named pipe (`\\.\pipe\<name>`). The radio must communicate with mpv on all three platforms without duplicating the protocol logic (command encoding, event parsing, request/response correlation).

## Decision Drivers

- mpv's IPC protocol is identical across platforms — only the transport differs
- The protocol logic (JSON-RPC framing, newline-delimited messages, request IDs, event parsing) is substantial and must not be duplicated
- Windows named pipes have different connection semantics (CreateFile vs connect) but provide the same Read + Write interface once connected
- The `polling` crate (ADR-009) needs a raw fd/handle to watch — the abstraction must expose this
- Platform-specific code should be minimal and isolated behind `#[cfg]` gates

## Considered Options

1. **Trait abstraction: `IpcStream: Read + Write` with platform-specific constructors** — Unix impl wraps `UnixStream`, Windows impl wraps a named pipe handle
2. **`interprocess` crate** — provides a cross-platform local socket abstraction out of the box
3. **Conditional compilation with duplicated MpvClient** — separate Unix and Windows versions of the whole IPC module
4. **TCP fallback** — use mpv's `--input-ipc-server=tcp://...` on all platforms (mpv doesn't actually support this)

## Decision Outcome

**Chosen option:** Trait abstraction (option 1) as the primary design, with the `interprocess` crate (option 2) as a candidate implementation for the Windows side. The protocol logic lives in a single `mpv.rs` module parameterized over the stream type; platform-specific code is confined to `platform.rs`.

### Positive Consequences

- The MpvClient struct is generic over the stream: `MpvClient<S: Read + Write>` — all protocol logic is shared
- `platform.rs` contains only the connection/retry code and the raw fd/handle extraction for `polling`
- Adding a new platform (e.g., FreeBSD) requires only a new `#[cfg]` block in `platform.rs`
- The abstraction is testable: unit tests can use an in-memory stream (e.g., `std::io::Cursor` or a pipe pair) to test JSON parsing without spawning mpv

### Negative Consequences

- Windows named pipe I/O may need the `interprocess` crate or raw `windows-sys` calls — this adds a platform-conditional dependency
- Non-blocking I/O semantics differ subtly between Unix sockets and Windows pipes (overlapped I/O vs O_NONBLOCK) — the abstraction must paper over this
- The raw fd/handle for `polling` registration must be exposed through a platform-specific method on the trait (e.g., `fn as_raw_fd() -> RawFd` on Unix, `fn as_raw_handle() -> RawHandle` on Windows), making the trait not purely platform-agnostic

## Implementation Notes

```rust
// platform.rs (simplified)

#[cfg(unix)]
pub type IpcStream = std::os::unix::net::UnixStream;

#[cfg(windows)]
pub type IpcStream = interprocess::local_socket::LocalSocketStream;
// or a raw named-pipe wrapper via windows-sys

pub fn connect(path: &str, retries: u32) -> io::Result<IpcStream> { ... }
```

The `MpvClient` then holds an `IpcStream` and a read buffer for partial-line accumulation. The decision on `interprocess` vs raw Win32 for the Windows implementation is deferred to Task 9.

## Links and References

- Spec: `.claude/specs/pstar-radio/requirements.md` (R4, R10, Task 5, Task 9)
- mpv IPC protocol: https://mpv.io/manual/master/#json-ipc
- ADR-009: Polling crate (the event loop that watches the IPC fd)
