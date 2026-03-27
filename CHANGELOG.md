# Changelog

## [0.2.0] - 2026-03-27

### 🐛 Bug Fixes

- **Auth status not refreshing after login** — Popup now checks auth on icon click instead of only at mount time. Users no longer need to restart the app after logging in via Settings. (`6c18ad0`, `c6032f5`)
- **Settings window not coming to front** — When Settings is behind other windows (e.g., Teams), clicking the ⚙ icon now reliably brings it to the foreground using Win32 `BringWindowToTop` + `SetForegroundWindow`. (`f2b6015`)
- **Popup triggering on read-only content** — Popup now only appears on editable elements (input, textarea, contenteditable). Selecting text on read-only web pages (GitHub, docs) no longer triggers the popup. (`d5d4e0e`, `1ca1fd8`)
- **Incompatible models causing errors** — Models that don't support the `/chat/completions` endpoint (e.g., `gpt-5.4-mini`) are now filtered out of the model picker, preventing HTTP 400 errors. (`9bd1913`)

### ✨ Enhancements

- **Model picker grouped by vendor** — Models are now organized under vendor headers (Anthropic, Google, OpenAI, etc.) with alphabetical sorting within each group. (`122d344`)
- **Model category indicator** — Powerful models (Opus, GPT-5.2-Codex, etc.) are marked with a ⚡ icon in the picker to indicate higher resource usage. (`d415540`)
- **Chat completions hint** — Added a note below the model dropdown explaining that only chat-completions-compatible models are listed. (`ff22b4f`)
- **Beast mode default ON** — Beast mode is now enabled by default for new installations. (`5adbad4`)
- **Quick access to Settings** — Model name and beast mode icon in the Popup bottom bar are now clickable buttons that open Settings. (`5adbad4`)

### 🔧 Internal

- Optimized CI build: limited bundle to NSIS, improved caching. (`170c5cd`)
- Removed unnecessary parentheses in `monitor.rs`. (`b3cd17b`)
- Documented SmartScreen warning and updater 404 in `issues.md`. (`0825f38`)

## [0.1.0] - 2026-03-26

- Initial release
