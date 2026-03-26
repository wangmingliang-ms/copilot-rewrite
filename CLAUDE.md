# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Copilot Rewrite is a **Windows-only system-level** text translation and polishing tool. When a user selects text in any application, a floating popup appears near the cursor. Clicking it calls the GitHub Copilot API to translate/polish the text, then the user can replace the original text in-place via simulated Ctrl+V.

## Build & Run Commands

```bash
# Install frontend dependencies
npm install

# Development (starts both Vite dev server and Tauri process)
npm run tauri dev

# Production build (outputs installer in src-tauri/target/release/bundle/)
npm run tauri build

# Build frontend only (outputs to dist/)
npm run build

# Run Rust tests (from src-tauri/)
cd src-tauri && cargo test

# Type-check frontend
npx tsc --noEmit

# Run a single Rust test
cd src-tauri && cargo test clipboard::manager::tests::test_clipboard_roundtrip
```

**Prerequisites**: Rust toolchain (1.77.2+), Node.js, WebView2 runtime (pre-installed on Windows 10/11).

## Architecture

### Two-Process Model (Tauri 2.0)

The app runs as a Rust process hosting a WebView2 frontend. They communicate via Tauri's IPC (`invoke` from JS, `#[tauri::command]` in Rust, and `emit`/`listen` for events).

### Windows: `popup` and `settings`

There are exactly **two Tauri webview windows** defined in `tauri.conf.json`:

- **`popup`** — The floating overlay. Starts as a 48×48 icon, transitions to spinning, then expands to show results. Uses `WS_EX_NOACTIVATE` to avoid stealing focus from the source app. Routes to `#/popup` in the React app.
- **`settings`** — A standard decorated window for login, model selection, and preferences. Routes to `#/settings`. Hidden on close (not destroyed), re-shown from tray.

The frontend (`src/components/App.tsx`) uses `window.location.hash` to decide which view to render — there is no React Router.

### Popup State Machine

The popup window transitions through states managed in `Popup.tsx`:

```
icon (48×48) → spinning (48×48) → expanded (auto-sized) → dismissed
                                  ↳ error
```

The **Rust overlay module** (`src-tauri/src/overlay/mod.rs`) handles window positioning, sizing, and toggling `WS_EX_NOACTIVATE` via Win32 API. Position is calculated once in `show_popup_icon()` and stored in statics — subsequent transitions reuse it to prevent jumping.

### Selection Detection Pipeline

A dedicated **OS thread** (not tokio) runs the selection monitor loop (`selection/monitor.rs`):

1. **UIA polling** (`selection/uia.rs`) — Calls `IUIAutomation::GetFocusedElement()` then `TextPattern.GetSelection()` every 100ms
2. **Debounce** — Waits 200ms after text stabilizes before showing popup
3. **Emit** — Fires `selection-detected` event to frontend, stores `SelectionInfo` in `AppState`
4. **Pause** — Stops polling when `preview_visible` is true or `enabled` is false
5. **Generation counter** — `selection_generation` AtomicU64 is bumped on dismiss to reset monitor state

COM is initialized per-thread (`CoInitializeEx`), so UIA must stay on its dedicated thread.

### Copilot API Token Exchange

The app uses a **two-step token flow** (`copilot/client.rs`):

1. GitHub personal token → `GET /copilot_internal/v2/token` → short-lived Copilot session token
2. Session token → `POST /chat/completions` with SSE streaming (assembled into full response)

Session tokens are cached in memory with expiry tracking. The GitHub token is obtained via **OAuth Device Flow** (`copilot/oauth.rs`) using VS Code's well-known client ID.

### Text Replacement

The replace flow (`replacement/engine.rs`) runs on a **dedicated OS thread** via `tokio::task::spawn_blocking`:

1. `SetForegroundWindow()` to restore focus to the source app (HWND stored from selection time)
2. Write result to clipboard via `clipboard-win`
3. `SendInput()` to simulate Ctrl+V
4. Monitoring is **temporarily disabled** for 800ms to prevent re-triggering

### Shared State (`AppState`)

All cross-module state lives in `Arc<AppState>` using `parking_lot::Mutex`:

- `enabled` — global on/off toggle
- `preview_visible` — pauses UIA polling while popup is expanded
- `current_selection` — the `SelectionInfo` including source HWND, app name, input element rect
- `selection_generation` — atomic counter bumped on dismiss for monitor reset
- `settings` — persisted to `%APPDATA%/copilot-rewrite/settings.json`

### Persistent Storage

All config files go under `%APPDATA%/copilot-rewrite/`:

- `settings.json` — user settings (model, language, beast_mode, etc.)
- `auth.json` — saved GitHub token + username
- `logs/YYYY-MM-DD.log` — date-rotated log files (INFO+ level)
- `replace-debug.log` — detailed replacement engine debug log
- `.lock` — single-instance enforcement (exclusive file lock)

### System Prompts

The Copilot client has **two prompt tiers** in `copilot/client.rs`:

- **Normal mode**: translate, polish, or translate+polish. The translate+polish mode returns JSON `{"reorganized": "...", "translated": "..."}` so the popup shows both versions.
- **Beast mode**: same three actions but with full creative rewrite freedom. Toggled via settings.

The frontend (`Popup.tsx`) parses the JSON response, renders both fields as Markdown, and lets users toggle between the reorganized original and the translated output.

### Key Technical Constraints

- **Focus management is critical**: The popup must not steal focus from the source app during icon/spinning states (or the selection is lost). `WS_EX_NOACTIVATE` is toggled on/off.
- **DPI awareness**: The overlay module converts between physical and logical pixels using per-monitor DPI (`GetDpiForMonitor`). Position is stored once to avoid DPI round-trip errors.
- **Input element rect**: When available from UIA, the popup aligns to the focused input's bounding rect rather than the mouse cursor, providing more stable positioning.
- **SendInput threading**: Must run on a proper OS thread, not a tokio async task, for input injection to work correctly.
