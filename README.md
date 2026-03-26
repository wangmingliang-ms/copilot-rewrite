# Copilot Rewrite

A **Windows-only system-level** text translation and polishing tool powered by GitHub Copilot. Select text in any application, click the floating popup, and get instant AI-powered translation, polishing, or both — then replace the original text in-place with a single click.

## How It Works

1. **Select text** in any application (Teams, Outlook, Edge, etc.)
2. A small **floating icon** appears near your cursor
3. **Click the icon** — it spins while calling the Copilot API
4. The popup **expands** to show the translated/polished result rendered as Markdown
5. Click **Replace** to paste the result back into the original application, or **Copy** to clipboard

The entire flow is non-intrusive: the popup never steals focus from the application you're working in.

## Features

- **Translate + Polish** — Auto-detects the source language, translates to your target language, and polishes the text in one step
- **Beast Mode** — Enable full creative rewriting with restructuring, examples, and best-version output
- **Multi-model support** — Choose from available Copilot models (GPT-4o, Claude Sonnet, etc.)
- **13 target languages** — English, Chinese (Simplified/Traditional), Japanese, Korean, French, German, Spanish, Portuguese, Russian, Arabic, Hindi, Italian
- **In-place replacement** — Simulates Ctrl+V to paste results directly into the focused application
- **Markdown rendering** — Results are rendered as rich Markdown with toggle to view raw source
- **Original comparison** — Expand the "Original (reorganized)" section to compare with the polished source
- **Regenerate** — Re-run the translation without dismissing the popup
- **System tray** — Runs in the background with enable/disable toggle
- **Auto-start** — Optionally launch on Windows login
- **Auto-update** — Built-in updater checks for and installs new versions
- **Date-rotated logs** — Detailed logs stored in `%APPDATA%/copilot-rewrite/logs/`

## Screenshots

<!-- TODO: Add screenshots of the popup icon, expanded result, and settings panel -->

## Prerequisites

- **Windows 10/11** (x64)
- **WebView2 Runtime** (pre-installed on Windows 10 21H2+ and Windows 11)
- **GitHub account** with [Copilot](https://github.com/features/copilot) access

### For Development

- [Rust toolchain](https://rustup.rs/) (1.77.2+)
- [Node.js](https://nodejs.org/) (LTS recommended)
- [Tauri CLI](https://v2.tauri.app/start/prerequisites/) prerequisites

## Installation

Download the latest installer from [Releases](https://github.com/wangmingliang-ms/copilot-rewrite/releases) and run it. The app will appear in your system tray.

### First-Time Setup

1. Right-click the tray icon → **Settings**
2. Click **Sign in with GitHub** — a device code flow opens in your browser
3. Authorize the app on GitHub
4. Select your preferred **AI model** and **target language**
5. Close settings — you're ready to go!

## Development

```bash
# Install frontend dependencies
npm install

# Development (starts both Vite dev server and Tauri process)
npm run tauri dev

# Production build (outputs installer in src-tauri/target/release/bundle/)
npm run tauri build

# Build frontend only (outputs to dist/)
npm run build

# Run Rust tests
cd src-tauri && cargo test

# Type-check frontend
npx tsc --noEmit

# Run a single Rust test
cd src-tauri && cargo test clipboard::manager::tests::test_clipboard_roundtrip
```

## Architecture

### Tech Stack

| Layer | Technology |
|-------|------------|
| Framework | [Tauri 2.0](https://v2.tauri.app/) (Rust + WebView2) |
| Frontend | React 19 + TypeScript + Tailwind CSS |
| Backend | Rust with `windows` crate for Win32/COM APIs |
| API | GitHub Copilot Chat Completions (SSE streaming) |
| Auth | GitHub OAuth Device Flow (VS Code client ID) |

### Two-Process Model

The app runs as a Rust process hosting a WebView2 frontend. They communicate via Tauri's IPC (`invoke` from JS, `#[tauri::command]` in Rust, and `emit`/`listen` for events).

### Windows

| Window | Purpose |
|--------|---------|
| `popup` | Floating overlay — starts as 48×48 icon, expands to show results. Uses `WS_EX_NOACTIVATE` to avoid stealing focus. |
| `settings` | Standard window for login, model selection, and preferences. Hidden on close, re-shown from tray. |

### Popup State Machine

```
icon (48×48) → spinning (48×48) → expanded (auto-sized) → dismissed
                                  ↳ error
```

### Backend Modules

```
src-tauri/src/
├── lib.rs              # App state, Tauri commands, entry point
├── main.rs             # Windows entry point
├── copilot/
│   ├── client.rs       # Copilot API client (token exchange, SSE streaming, system prompts)
│   └── oauth.rs        # GitHub OAuth Device Flow (login, token persistence)
├── selection/
│   ├── monitor.rs      # Selection detection loop (dedicated OS thread)
│   └── uia.rs          # UI Automation TextPattern polling
├── overlay/
│   └── mod.rs          # Win32 window management (positioning, DPI, WS_EX_NOACTIVATE)
├── replacement/
│   └── engine.rs       # Text replacement (SetForegroundWindow + SendInput)
├── clipboard/
│   ├── mod.rs          # Clipboard read/write wrappers
│   └── manager.rs      # Clipboard content management
├── tray/
│   └── mod.rs          # System tray icon and menu
└── autostart/
    └── mod.rs          # Windows startup registry management
```

### Frontend Components

```
src/
├── main.tsx                    # React entry point
├── components/
│   ├── App.tsx                 # Hash-based routing (popup vs settings)
│   ├── Popup.tsx               # Popup state machine (icon → spinning → expanded)
│   ├── SettingsPanel.tsx       # Login, model, language, beast mode settings
│   ├── Toolbar.tsx             # Action toolbar
│   ├── Preview.tsx             # Result preview
│   └── LoginDialog.tsx         # GitHub OAuth login UI
├── hooks/
│   ├── useSelection.ts        # Selection event listener
│   └── useUpdater.ts          # Auto-update hook
└── styles/
    └── index.css               # Tailwind base styles
```

### Key Design Decisions

- **Dedicated OS threads** — UIA COM polling and `SendInput` run on native OS threads (not tokio), as required by Windows COM/input APIs
- **Focus management** — The popup uses `WS_EX_NOACTIVATE` during icon/spinning states to avoid stealing focus (which would lose the text selection in the source app)
- **DPI awareness** — Per-monitor DPI conversion using `GetDpiForMonitor`, with position stored once to avoid round-trip errors
- **Generation counter** — `AtomicU64` bumped on dismiss to reset monitor state without race conditions
- **800ms monitoring pause** — After text replacement, monitoring is disabled briefly to prevent re-triggering on the pasted text

## Configuration

Settings are stored in `%APPDATA%/copilot-rewrite/settings.json`:

```json
{
  "target_language": "English",
  "auto_start": false,
  "beast_mode": false,
  "model": "claude-sonnet-4",
  "poll_interval_ms": 100
}
```

Auth credentials are stored separately in `%APPDATA%/copilot-rewrite/auth.json`.

## License

MIT
