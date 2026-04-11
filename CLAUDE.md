# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

Copilot Rewrite is a **Windows-only system-level** text translation and polishing tool. When a user selects text in any application, a floating popup appears near the cursor. Clicking it calls the GitHub Copilot API to translate/polish the text. The app has two modes:

- **Write Mode** — Triggered on input elements (`<input>`, `<textarea>`, `contentEditable`). Polishes + translates text. User can replace original text in-place via simulated Ctrl+V.
- **Read Mode** — Triggered on non-input elements (webpage text, PDFs). Translates and optionally summarizes. Result is display-only (Copy, no Replace).

Design rules from the project owner are in `SPEC.md` — read it before making behavioral changes to popup lifecycle or selection detection.

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

### Windows

There are **three Tauri webview windows** defined in `tauri.conf.json`:

- **`splashscreen`** — 320x280 transparent splash, always-on-top, auto-closes after 1.5s.
- **`popup`** — The floating overlay. Starts as a 48x48 icon, transitions to spinning, then expands to show results. Uses `WS_EX_NOACTIVATE` to avoid stealing focus. Routes to `#/popup` in the React app.
- **`settings`** — Standard decorated window for login, model selection, and preferences. Routes to `#/settings`. Hidden on close (not destroyed), re-shown from tray.

The frontend (`src/components/App.tsx`) uses `window.location.hash` to decide which view to render — there is no React Router.

### Popup State Machine

```
icon (48×48) → spinning (48×48) → expanded (auto-sized) → dismissed
                                  ↳ error
```

The **Rust overlay module** (`src-tauri/src/overlay/mod.rs`) handles window positioning, sizing, and toggling `WS_EX_NOACTIVATE` via Win32 API. Position is calculated once in `show_popup_icon()` and stored in statics — subsequent transitions reuse it to prevent jumping.

### Write Mode vs Read Mode

Mode is determined by `SelectionInfo.is_input_element` (set by `UiaEngine::is_editable_element()` in the monitor loop):

| Aspect | Write Mode (`is_input_element: true`) | Read Mode (`is_input_element: false`) |
|--------|--------------------------------------|--------------------------------------|
| Trigger | Input fields | Webpage text, PDFs |
| Action | `TranslateAndPolish` | `ReadModeTranslate` |
| LLM response | JSON `{"reorganized": "...", "translated": "..."}` | JSON `{"summary": "...", "translation": "..."}` or plain text |
| Popup buttons | Replace, Copy, Refresh, Markdown toggle, Creative mode | Copy, Refresh, Dismiss (no Replace) |
| Sub-modes | Normal / Creative Mode | Translate+Summarize / Simple Translate |
| Collapsible section | "Original (reorganized)" | "Full Translation" (when summarize mode) |

Read Mode is toggled via `Settings.read_mode_enabled`. Its sub-mode (`translate_summarize` or `simple_translate`) is in `Settings.read_mode_sub`.

### Selection Detection Pipeline

A dedicated **OS thread** (not tokio) runs the selection monitor loop (`selection/monitor.rs`):

1. **UIA polling** (`selection/uia.rs`) — Calls `IUIAutomation::GetFocusedElement()` then `TextPattern.GetSelection()` every 100ms
2. **Mode detection** — `get_selected_text_any()` returns `(Option<String>, bool)` where the bool is `is_input_element`. If Read Mode is disabled and the element is non-input, the selection is ignored.
3. **Debounce** — Waits 200ms after text stabilizes before showing popup
4. **Emit** — Fires `selection-detected` event to frontend, stores `SelectionInfo` in `AppState`
5. **Pause** — Stops polling when `preview_visible` is true or `enabled` is false
6. **Generation counter** — `selection_generation` AtomicU64 is bumped on dismiss to reset monitor state

COM is initialized per-thread (`CoInitializeEx`), so UIA must stay on its dedicated thread.

### IPC Communication

**Frontend → Backend (invoke):** `process_and_show_preview`, `dismiss_popup`, `cancel_request`, `replace_text`, `copy_to_clipboard`, `copy_html_to_clipboard`, `resize_popup_content`, `get_settings`, `update_settings`, `get_auth_status`, `start_github_login`, `poll_github_login`, `logout`, `list_models`, `open_settings`, `log_action`

**Backend → Frontend (events):** `selection-detected`, `selection-cleared`, `show-preview-loading`, `show-preview-result` (carries `ProcessResponse`), `show-preview-error`, `request-cancelled`

### Copilot API Token Exchange

The app uses a **two-step token flow** (`copilot/client.rs`):

1. GitHub personal token → `GET /copilot_internal/v2/token` → short-lived Copilot session token
2. Session token → `POST /chat/completions` with SSE streaming (assembled into full response)

Session tokens are cached in memory with expiry tracking. The GitHub token is obtained via **OAuth Device Flow** (`copilot/oauth.rs`) using VS Code's well-known client ID.

### System Prompts

The Copilot client has **three prompt tiers** in `copilot/client.rs`:

- **Normal mode** (Write): translate, polish, or translate+polish. The translate+polish mode returns JSON `{"reorganized": "...", "translated": "..."}`.
- **Creative mode** (Write): same three actions but with full creative rewrite freedom. Toggled via popup toolbar.
- **Read mode**: `read_mode_translate_prompt` (faithful translation, plain text output) and `read_mode_translate_summarize_prompt` (returns JSON `{"summary": "...", "translation": "..."}`).

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
- `current_selection` — the `SelectionInfo` including source HWND, app name, `is_input_element`
- `selection_generation` — atomic counter bumped on dismiss for monitor reset
- `settings` — persisted to `%APPDATA%/copilot-rewrite/settings.json`

### Persistent Storage

All config files go under `%APPDATA%/copilot-rewrite/`:

- `settings.json` — user settings (model, languages, creative_mode, read_mode_enabled, read_mode_sub, etc.)
- `auth.json` — saved GitHub token + username
- `logs/YYYY-MM-DD.log` — date-rotated log files (INFO+ level)
- `replace-debug.log` — detailed replacement engine debug log
- `.lock` — single-instance enforcement (exclusive file lock)

### Frontend Patterns

- **State management**: Local React state only (`useState`/`useCallback`). No global store. Settings persist to Rust backend via `invoke("update_settings")` with 300ms debounce auto-save.
- **Styling**: Tailwind CSS 3 with class-based dark mode. Custom brand colors in `tailwind.config.js` (`copilot-blue`, etc.). GitHub Markdown CSS with dark mode overrides in `src/styles/index.css`.
- **Markdown rendering**: Uses `marked` library → HTML string → `dangerouslySetInnerHTML`. Both Write Mode and Read Mode results are rendered as Markdown.
- **Auto-update**: `useUpdater` hook uses `@tauri-apps/plugin-updater` to check GitHub releases for updates, with download progress and install support.

### Key Technical Constraints

- **Focus management is critical**: The popup must not steal focus from the source app during icon/spinning states (or the selection is lost). `WS_EX_NOACTIVATE` is toggled on/off.
- **DPI awareness**: The overlay module converts between physical and logical pixels using per-monitor DPI (`GetDpiForMonitor`). Position is stored once to avoid DPI round-trip errors.
- **SendInput threading**: Must run on a proper OS thread, not a tokio async task, for input injection to work correctly.
- **COM threading**: UIA operations must stay on the dedicated OS thread where `CoInitializeEx` was called.
- **Dismiss rule** (from SPEC.md): "Selection gone = Popup gone." Dismiss is unconditional — never gated on popup state (icon/spinning/expanded).
