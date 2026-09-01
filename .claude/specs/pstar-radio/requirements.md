# Requirements Document

## Introduction

Writers work better with background music. `ytr` (YouTube radio) is an Emacs Lisp package by Alvaro Ramirez that streams audio from YouTube channels and playlists via `yt-dlp` + `mpv` — a minimal, keyboard-driven jukebox for focused work. This spec defines a Rust port of ytr's core functionality as a standalone TUI binary (`pstar-radio`) within the PerfectStar 2k workspace, with an integration hook (`^OY`) in the editor.

### Prior Art

- [xenodium/ytr](https://github.com/xenodium/ytr) — the Emacs package being ported (~1600 LOC Elisp)
- Core ytr features: add/remove YouTube channels by URL, fetch track listings via yt-dlp, play audio via mpv (IPC over Unix socket), persist state, play/pause/next/prev/seek, marquee title scrolling, auto-advance

### Personas

- **Nadia — the novelist.** Wants ambient background music while writing long sessions. Needs a single keystroke to launch/dismiss the radio without losing her place in the manuscript.
- **Marcus — the non-fiction author.** Prefers curated playlists of lo-fi or classical. Wants to configure preset channels in a config file and cycle between them.
- **Priya — the journalist.** Wants a timer-friendly ambient tool. Launches radio at the start of a sprint, forgets about it, quits when done.

### Design Constraints

- **DC1 — Workspace member.** `pstar-radio` lives in the same Cargo workspace as `pstar`, sharing dependencies and the theme system via a `pstar-common` crate.
- **DC2 — External dependencies.** Requires `yt-dlp` and `mpv` in PATH. These are runtime, not build-time, dependencies. The binary degrades gracefully when either is absent.
- **DC3 — Cross-platform IPC.** mpv communicates via Unix domain sockets (macOS/Linux) or named pipes (Windows). Both must be supported.
- **DC4 — No async runtime.** Uses the `polling` crate for a lightweight event loop, consistent with pstar's synchronous architecture.
- **DC5 — Ambient UX.** The radio is background accompaniment for writing. The interface is minimal: a status bar with now-playing info and a toggle overlay for controls. It should never demand attention.
- **DC6 — Shared theming.** Visual appearance uses pstar's existing theme system (wp-blue, wordstar, terminal-default), extracted into `pstar-common`.

### Out of Scope

- Video playback or thumbnails (audio only)
- Streaming sources other than YouTube
- Embedding the radio's full UI inline within pstar's TUI (future work; this spec covers shell-out via `^OY`)
- Mobile or web interfaces
- User authentication / private YouTube content
- Shuffle / random mode (future addition)

## Glossary

- **Channel**: A YouTube channel or playlist treated as an ordered collection of tracks.
- **Track**: A single YouTube video treated as an audio stream (id, title, duration, URL, channel membership).
- **State**: The persisted catalog of channels and their tracks, plus last-played position, saved as JSON.
- **IPC**: Inter-process communication with mpv via JSON-RPC over a Unix domain socket or Windows named pipe.
- **Overlay**: The full-screen player view showing controls and track info (toggled on/off from the status bar view).

## Requirements

### Requirement 1: Channel Management

**User Story:** As Marcus (non-fiction author), I want to add YouTube channels and playlists by URL so that I can build a curated catalog of background music without leaving the terminal.

#### Acceptance Criteria

1. THE Channel_System SHALL let the user add a YouTube channel or playlist by URL
2. WHEN a channel is added, THE Channel_System SHALL fetch its track listing via `yt-dlp --flat-playlist --dump-single-json` and store it in state
3. THE Channel_System SHALL distinguish channels (append `/videos` to URL) from playlists (use URL as-is) using query-string detection
4. THE Channel_System SHALL let the user remove a channel and all its tracks from the catalog, with confirmation
5. IF `yt-dlp` is not in PATH, THEN THE Channel_System SHALL display a clear error and refuse the operation
6. Channel metadata SHALL include: id, display name, URL, kind (channel/playlist), and an ordered list of track ids

### Requirement 2: Track Browsing and Selection

**User Story:** As Nadia (novelist), I want to browse and filter all available tracks so that I can quickly find something to listen to and get back to writing.

#### Acceptance Criteria

1. THE Track_Browser SHALL present all tracks across all channels in a scrollable, filterable list
2. THE Track_Browser SHALL show each track as "Channel Name - Track Title" with duration
3. WHEN the user selects a track, THE Track_Browser SHALL begin playback immediately
4. THE Track_Browser SHALL provide a channel-selection view that plays the first track of the chosen channel
5. Track ordering SHALL be channel-then-listing-order (the order yt-dlp returns them)

### Requirement 3: Playback Control

**User Story:** As Priya (journalist), I want simple play/pause/skip controls so that music stays in the background without interrupting my writing flow.

#### Acceptance Criteria

1. THE Playback_Engine SHALL play audio via `mpv --no-video --force-window=no` with IPC enabled
2. THE Playback_Engine SHALL support: play, pause, toggle, next track, previous track, seek forward, seek backward
3. WHEN a track ends normally (mpv exit 0), THE Playback_Engine SHALL auto-advance to the next track in catalog order
4. IF there is no next track, THEN playback SHALL stop gracefully
5. THE Playback_Engine SHALL display elapsed time (polled from mpv each second) and total duration
6. Seek SHALL default to 5 seconds; a "large seek" SHALL default to 60 seconds (configurable)
7. IF `mpv` is not in PATH, THEN THE Playback_Engine SHALL display a clear error at startup or first play attempt
8. IF mpv reports a playback error, THEN THE Playback_Engine SHALL display the error and stop (not crash)

### Requirement 4: mpv IPC

**User Story:** As a developer maintaining pstar-radio, I want reliable bidirectional communication with mpv so that the player state is always accurate and commands are responsive.

#### Acceptance Criteria

1. THE IPC_Client SHALL connect to mpv via a Unix domain socket (macOS/Linux) or named pipe (Windows)
2. Connection SHALL retry with backoff (50ms intervals, up to 2 seconds) since the socket appears after mpv spawns
3. THE IPC_Client SHALL observe mpv properties: `pause`, `core-idle`, and poll `time-pos`
4. THE IPC_Client SHALL handle mpv events: `property-change`, `end-file` (including error reason)
5. The IPC stream SHALL be set to non-blocking after connection, read via the polling event loop
6. IF the IPC connection drops unexpectedly, THEN THE IPC_Client SHALL treat playback as stopped

### Requirement 5: State Persistence

**User Story:** As Nadia (novelist), I want pstar-radio to remember my channels and where I left off so that I can quit and resume later without rebuilding my catalog.

#### Acceptance Criteria

1. THE State_Manager SHALL persist state as JSON at `<data_local_dir>/pstar-radio/state.json`
2. State SHALL include: a version field, all channels with their tracks, and last-played track id
3. Saves SHALL be atomic (write temp file, rename)
4. WHEN the app launches, THE State_Manager SHALL load state and resume from the last-played track (ready to play, not auto-playing)
5. IF the state file is missing or corrupt, THEN THE State_Manager SHALL start with an empty catalog (no crash)

### Requirement 6: User Interface

**User Story:** As any writer using pstar, I want the radio to look and feel like part of the same application so that switching between editor and radio is visually seamless.

#### Acceptance Criteria

1. THE UI_System SHALL render a TUI using ratatui with the shared pstar theme (wp-blue default)
2. THE UI_System SHALL show a status bar (bottom line) with: play-state icon, track title (marquee-scrolling if wider than available columns), elapsed/duration
3. THE UI_System SHALL show a main area with: "now playing" card (track, channel, controls hint) or the active list (tracks/channels) when browsing
4. THE UI_System SHALL show a controls hint line with available keys
5. Marquee scrolling SHALL bounce (scroll to end, pause, scroll back, pause) at 4 ticks/second
6. Play-state icons SHALL be Unicode: `▶` playing, `⏸` paused, `■` stopped, `◁◁` previous, `▷▷` next
7. THE UI_System SHALL support all three pstar themes, selectable in config

### Requirement 7: Key Bindings

**User Story:** As a keyboard-driven writer, I want simple single-key controls that don't require chords so that I can manage playback without thinking.

#### Acceptance Criteria

1. THE Keymap SHALL bind `space` to toggle play/pause (resume last-played if stopped)
2. THE Keymap SHALL bind `n` to next track and `p` to previous track
3. THE Keymap SHALL bind `f` to seek forward (5s) and `b` to seek backward (5s)
4. THE Keymap SHALL bind `+` to add channel (prompts for URL) and `-` to remove channel (shows list, confirms)
5. THE Keymap SHALL bind `/` to pick a track (filterable list) and `c` to pick a channel to play
6. THE Keymap SHALL bind `o` to open current track in browser
7. THE Keymap SHALL bind `q` to quit (stop playback, save state, exit)

### Requirement 8: Configuration

**User Story:** As Marcus (non-fiction author), I want to preconfigure my favorite channels in a config file so that they're always available without manual URL entry.

#### Acceptance Criteria

1. THE Config_System SHALL read config from `<config_dir>/pstar-radio/config.toml`
2. Configurable fields SHALL include: `theme` (string), `channels` (array of `{ name, url }`), `seek_seconds` (default 5), `seek_large_seconds` (default 60)
3. Channels in config SHALL be auto-fetched on first launch if not already in state
4. Config channels SHALL be "pinned" — not removable via the `-` command
5. WHERE no config file exists, THE Config_System SHALL use defaults and operate normally

### Requirement 9: Integration with pstar Editor

**User Story:** As Nadia (novelist), I want to launch the radio from inside pstar with a single chord so that I never leave my writing environment to manage music.

#### Acceptance Criteria

1. THE pstar editor SHALL gain a `Cmd::Radio` command bound to `^OY` ("YouTube radio")
2. WHEN `^OY` is pressed, THE Integration SHALL: suspend the terminal, launch `pstar-radio` as a child process, and restore the terminal on return
3. The command SHALL appear in the command palette with the name "YouTube radio"
4. IF `pstar-radio` is not found in PATH (or adjacent to the pstar binary), THEN THE Integration SHALL display "pstar-radio not found" in the status bar
5. Playback state persists across invocations (via the state file), so quitting the radio and re-invoking it later resumes where the user left off

### Requirement 10: Cross-Platform Support

**User Story:** As a developer, I want pstar-radio to work on macOS, Linux, and Windows so that the full PerfectStar 2k experience is available everywhere.

#### Acceptance Criteria

1. On macOS/Linux: mpv IPC SHALL use Unix domain sockets in a temp directory
2. On Windows: mpv IPC SHALL use named pipes (`\\.\pipe\pstar-radio-mpv-<random>`)
3. Platform-specific code SHALL be isolated behind a trait (`IpcStream: Read + Write`) with `#[cfg]` implementations
4. The "open in browser" command SHALL use `open` (macOS), `xdg-open` (Linux), or `start` (Windows)

## Dependencies (New)

| Crate | Purpose | Used by |
|-------|---------|---------|
| `polling` | Lightweight event loop (fd watching) | pstar-radio |
| `serde` + `serde_json` | JSON parsing (yt-dlp output, mpv IPC, state file) | pstar-radio (already in workspace) |
| `toml` | Config file parsing | pstar-radio (already in workspace) |
| `dirs` | Platform config/data/state directories | pstar-radio (already in workspace) |
| `ratatui` + `crossterm` | TUI rendering and terminal I/O | pstar-radio (already in workspace) |
| `interprocess` or equivalent | Windows named pipe support | pstar-radio (Windows only) |

## Open Questions

1. **`interprocess` vs raw Win32 for Windows pipes.** The `interprocess` crate provides a portable abstraction but adds a dependency. Raw Win32 via `windows-sys` is zero-dep but more code. Decide during Task 9.
2. **Refresh of channel listings.** ytr re-fetches on demand. Should pstar-radio auto-refresh on launch, or only when the user explicitly requests? Current spec: manual only (user presses `+` on an existing channel to refresh).
3. **Embedded radio (future).** The current integration is shell-out. A future version could embed the radio as an in-process overlay within pstar's event loop (sharing the terminal). This would require the radio logic to be a library crate, not just a binary. The workspace structure supports this evolution.

## Traceability

- This spec originates from a feasibility assessment of porting [xenodium/ytr](https://github.com/xenodium/ytr) to Rust.
- Requirements R1–R10 are individually traceable to implementation tasks in `tasks.md`.
- The `^OY` integration (R9) connects this feature to the main pstar editor without coupling their implementations.
- Related ADRs: ADR-008 (workspace restructure), ADR-009 (polling crate), ADR-010 (cross-platform mpv IPC), ADR-011 (shell-out integration).
