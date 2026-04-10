# Changelog

## [0.11.0] - 2026-04-10

### ⚡ Performance

- **Read Mode: switched from JSON to separator format** — Read Mode LLM output now uses plain-text separators (`---VOCABULARY---` / `---SUMMARY---`) instead of JSON. Translation streams immediately as plain text — no more waiting for valid JSON structure. Eliminates the fragile 3-level JSON fallback parser (JSON.parse → newline fixer → extractJsonStringValue).
- **UIA detection: cached GetFocusedElement()** — The cross-process COM call `GetFocusedElement()` (1-10ms) is now called once per poll and reused across all 3 detection phases, saving 2-20ms per iteration.
- **Overlay: deduplicated popup positioning** — Extracted shared `compute_expanded_position()` helper, eliminating ~50 lines of duplicated code between `expand_popup()` and `expand_popup_streaming()`. Fixed redundant `get_scale_at()` calls (2 Win32 syscalls saved per expand).
- **Overlay: cached popup WebviewWindow** — All overlay functions now use a cached `WebviewWindow` static instead of HashMap lookup per call.
- **Monitor: reduced allocations on hot path** — `text.trim()` comparison now uses a borrowed `&str` slice; `to_string()` only happens when text actually changes.
- **Monitor: throttled preview_visible stuck check** — Window visibility query reduced from every loop iteration to every 5 seconds.
- **Single-pass `estimate_height()`** — Height estimation now counts chars and newlines in one loop instead of two `.chars()` iterations.
- **TCP keepalive on HTTP client** — Added 30s TCP keepalive to the reqwest client to prevent idle connection drops.

### 🔧 Improvements

- **Safer COM event handler** — Rewrote the manual COM vtable's `clone()`+`forget()` pattern to use `ManuallyDrop` for clearer ownership semantics and no risk of double-release.
- **Removed health check eval()** — The 5-minute health check no longer calls `popup.eval()` (which wrote `window.__healthcheck` that nothing read and could block the COM thread if WebView2 was unresponsive).
- **Cached debug_log file handle** — Replacement engine's debug log now uses a `thread_local!` cached file handle instead of opening the file on every call.
- **Removed unused `once_cell` dependency** — Dropped direct `once_cell` dependency from Cargo.toml (not used in source code).
- **Safety comment on `build_cf_html`** — Added doc comment explaining the 10-char placeholder ↔ `{:010}` format width invariant.
- **Removed dead `extract_translated()` function** — JSON-based text extraction for height estimation was dead code after the separator format switch.
- **All AUDIT items resolved** — 19 of 20 audit findings now fixed (only #1 SSE streaming architecture remains as TODO).

### 🧪 Tests

- **33 new separator parsing tests** — `tests/readModeSeparator.test.ts` covers `parseVocabularyLines` (15 tests) and `parseReadModeSeparator` (18 tests) for all section combinations, streaming partial text, CJK content, and edge cases.

## [0.10.0] - 2026-04-10

### ⚡ Performance

- **SSE streaming for real-time token delivery** — The Copilot API response is now streamed incrementally via SSE chunks instead of buffering the entire response. Text appears token-by-token as the LLM generates it, reducing perceived latency from 2-8s to ~200ms time-to-first-token.
- **Dramatically simplified system prompts** — All 7 prompts (6 write + 1 read) reduced to ~40-50% of original size. Removed verbose multi-step thinking chains, detailed structure/formatting sections, and unnecessary instructions. Shorter prompts = faster TTFT.
- **Write Mode: dropped JSON output format** — Switched from `{"reorganized":"...","translated":"..."}` JSON to a plain-text `---TRANSLATED---` separator format. The model can now start streaming immediately without planning JSON structure first.
- **Read Mode: dropped 4-mode auto-detection** — Previously the model had to reason about which mode to use (word/simple/complex/long) before outputting anything. Now always returns a fixed `{translation, summary?, vocabulary?}` shape — frontend decides layout from field presence.

### ✨ Features

- **Streaming "Thinking" phase** — In Write Mode (TranslateAndPolish), the reorganized/polished text streams first with a dimmed "Thinking..." indicator, then the translated text appears normally after the separator. Similar to the "thinking" display in AI chat interfaces.
- **Streaming vocabulary display** — Vocabulary entries in Read Mode now appear incrementally during streaming, not just after the full response completes.
- **UTF-8 safe streaming** — Raw byte buffer correctly handles multi-byte character boundaries (e.g. Chinese characters split across SSE chunks), eliminating garbled text during streaming.

### 🔧 Improvements

- **Stronger anti-answer constraint** — All prompts now include a prominent "CRITICAL" instruction at the top that explicitly prevents the LLM from answering questions, executing tasks, or responding to content in the selected text. The user text is always treated as text to be translated/polished, never as a prompt.
- **Increased popup offset from selection** — Popup icon now appears 4px further from the selected text (X: 8→12px, Y: 16→20px) to reduce accidental clicks when right-clicking.
- **Simplified Read Mode UI** — Removed the 4-mode rendering (word/simple/complex/long). Now uses 3 clean layouts derived from response content: simple (translation only), withVocab (translation + vocabulary), withSummary (summary + collapsible full translation).

### 🐛 Bug Fixes

- **Fixed stream timeout after ~30s** — Replaced the blanket 30s `timeout()` on the HTTP client with `connect_timeout(15s)` + `read_timeout(60s)`, so long-running streams no longer get killed mid-flight.
- **Fixed popup height too small during streaming** — First streaming chunk no longer determines popup height. Uses a fixed 250px streaming height, then resizes to fit final content.

## [0.9.2] - 2026-04-08

### 🐛 Bug Fixes

- **Fixed popup invisible after Windows sleep/resume** — After the computer sleeps for hours or days, the popup icon could become invisible even though the backend correctly detected text selections. Root cause: Tauri's `window.show()` silently fails when the WebView2 renderer enters a bad state after sleep/resume. Now uses Win32 `ShowWindow(SW_SHOWNOACTIVATE)` + `SetWindowPos(HWND_TOP)` directly, with `IsWindowVisible` verification and Tauri fallback.
- **Fixed popup model display not updating after Settings change** — Changing the model in Settings was saved correctly to the backend, but the Popup window still displayed the old model name. The Popup now refreshes settings (including the model name) on every icon click and on every new text selection event.

### 🔧 Improvements

- **Periodic popup health check** — A background health check runs every 5 minutes to verify the popup window is still functional: validates the HWND via `IsWindow`, and pings the WebView2 renderer via `eval()`. Logs `ERROR` when the window or renderer is in a bad state, enabling faster diagnosis of long-running stability issues.

## [0.9.1] - 2026-03-31

### 🐛 Bug Fixes

- **Fixed browser input areas incorrectly entering Read Mode** — When selecting text in browser input fields (e.g. GitHub review comment boxes), the tree-walker strategy could return `is_input=false` before the focused-element strategy corrected it to `is_input=true`. Since the monitor only updated `is_input` when the text content changed, the incorrect Read Mode flag would persist. Now the monitor also detects mode changes (`is_input` flips) even when the selected text is unchanged, and re-triggers the popup with the correct Write Mode.

## [0.9.0] - 2026-03-31

### ⚡ Performance

- **Eliminated blocking `refreshSettings` on icon click** — Previously, clicking the popup icon would `await refreshSettings()` which made two sequential IPC calls (`get_settings` + `list_models`) before sending the LLM request. The `list_models` call hit the GitHub API, adding hundreds of milliseconds of latency. Now removed entirely — LLM request fires immediately on click.
- **Replace delay reduced from ~1500ms to ~310ms** — Rewrote the text replacement pipeline:
  - Window activation: replaced fixed 300ms sleep with a poll loop that verifies `GetForegroundWindow` matches the target (typically completes in 5-10ms, max 200ms timeout)
  - Clipboard write: removed unnecessary 100ms sleep (Windows clipboard APIs are synchronous)
  - SendInput (Ctrl+V): removed unnecessary 300ms sleep (input queue injection is synchronous)
  - Re-enable monitoring cooldown: reduced from 800ms to 300ms
- **Added `[PERF]` performance instrumentation** — Timestamped logs in `process_and_show_preview` track each stage: loading event emission, LLM future creation, LLM response, popup expansion, and result emission.
- **Added `[PERF-LLM]` instrumentation in Copilot client** — Tracks Copilot token acquisition, HTTP request send, response headers, body read, and parsing completion times in the Write Mode `process()` method.

## [0.8.3] - 2026-03-30

### 🔧 Improvements

- **Two-phase Write/Read Mode detection** — Restructured UIA text selection detection into two clear phases. Phase 1 checks the focused element for editability (Write Mode). Phase 2 falls back to event/cached/point/tree strategies with `is_editable_element` checks on each result.
- **TreeWalker prefers editable elements** — When traversing the UIA subtree, editable child elements with selection are now preferred over non-editable parents, improving Write Mode detection in apps with nested UIA trees.
- **Event handler no longer blocks fallback strategies** — When the TextSelectionChanged event fires but the event element has no text, detection now falls through to subsequent strategies instead of returning early.

### 🐛 Bug Fixes

- **Fixed Write Mode detection in Teams input boxes** — Teams CKEditor input fields are now correctly identified as Write Mode via Phase 1 focused element check.
- **Fixed false Write Mode on browser webpages** — Restored `ValuePattern.IsReadOnly` check in `is_editable_element` to correctly classify read-only Document elements (e.g. Chrome/Edge webpages) as Read Mode.

### 📝 Known Issues

- **Feishu (Lark) input boxes detected as Read Mode** — Feishu's Electron-based UI exposes a single `Chrome_RenderWidgetHostHWND` Document element with `ReadOnly=true` for both input areas and message lists. UIA cannot distinguish between them, so Feishu input boxes are currently classified as Read Mode.

## [0.8.2] - 2026-03-29

### 🐛 Bug Fixes

- **Popup now hides when switching windows** — Previously, the popup icon would stay visible when switching from one app to another (e.g. Teams → Browser). Now uses process ID comparison to reliably detect window switches across all apps, including Electron-based apps like Teams.
- **Seamless cross-app popup experience** — When switching between apps that both have selected text, the popup smoothly transitions: the old app's popup hides and the new app's popup appears at the correct position.

## [0.8.1] - 2026-03-29

### 🔧 Improvements

- **Dynamic fold labels in Read Mode** — The collapsible "Full Translation" section now shows the actual translation language (e.g. "English (Full Translation)" or "Chinese (Full Translation)") instead of a static label. Matches Write Mode's dynamic language labels.
- **Popup icon default position** — Changed default popup icon position from Top Center to Top Left.
- **Reduced minimum popup height** — Minimum expanded popup height reduced from 160px to 120px for shorter content.

### 🐛 Bug Fixes

- **Read Mode long mode translation content** — Fixed an issue where the "Full Translation" fold could contain the original text instead of the translated version.

## [0.8.0] - 2026-03-29

### ✨ Features

- **Splash screen** — A branded launch screen now appears when the app starts, confirming it's loading. Auto-dismisses with a smooth fade-out after 1.5 seconds.
- **Popup icon position setting** — Choose where the popup icon appears relative to selected text: Top Left, Top Center (default), Top Right, Bottom Left, Bottom Center, or Bottom Right. Visual position picker in Settings.
- **Copy toast notification** — A "✓ Copied" toast briefly appears after copying text, confirming the action.

### 🔧 Improvements

- **Settings redesigned as Read/Write Assistants** — Settings now organized into 📖 Read Assistant and ✍️ Write Assistant, each with their own Target Language. Read Assistant translates to your mother tongue; Write Assistant translates to your output language. Beast Mode is now a sub-setting under Write Assistant.
- **Smart popup sizing** — Popup width now matches the width of your selected text (minimum 400px), instead of a fixed width.
- **Smart popup positioning** — Expanded popup appears directly above or below your selected text (based on available screen space), aligned with the selection — not at the mouse cursor.
- **Smarter vocabulary display** — Vocabulary highlights only appear when reading foreign language text. Selecting text in your native language skips vocabulary (you already know those words).
- **Update notification** — Update detection now auto-opens the Settings window so you can see the update banner immediately.

## [0.7.0] - 2026-03-29

### ✨ Features

- **Read Mode** — Select any non-input text (messages, articles, docs) for instant translation and explanation. AI auto-detects content type and applies one of four smart modes: 📖 Word (dictionary-style definition + examples), 💬 Simple (clean translation), 📚 Complex (translation + vocabulary highlights), 📋 Summary (key points + collapsible full translation).
- **Smart language direction** — AI auto-detects source language and translates to the appropriate target. Select English → get Chinese translation; select Chinese → get English translation. No manual toggle needed.
- **Read Mode settings** — New Settings section to toggle Read Mode on/off and configure native/target languages.

### ⚡ Performance

- **Event-driven text selection detection** — Selection changes are now detected via UIA `TextSelectionChanged` events instead of pure polling, resulting in faster response times and lower CPU usage.

### 🔧 Improvements

- **Write Mode language direction** — `Polished` output now consistently uses your native language, `Translated` output consistently uses your target language. Previously the direction could be inconsistent depending on input.
- **Dynamic fold tab labels** — Collapse labels now show your configured native language (e.g. "Chinese (Polished)") instead of hardcoded text.

## [0.6.0] - 2026-03-27

### ✨ Enhancements

- **Dark mode support** — Full dark mode across Popup and Settings windows. Follows system preference by default, or manually set to Light/Dark in Settings → Appearance. Covers all UI states: icon, spinning, expanded, error, markdown preview, action bar, scrollbars, and code blocks. (`74dd903`)

### 🐛 Bug Fixes

- **Popup dark mode not syncing with Settings** — Popup and Settings are separate Tauri windows with independent DOMs. Changing theme in Settings now correctly propagates to the Popup on next appearance. Beast mode icon dark background and spinning state hover colors also fixed. (`d96d395`)
- **Markdown bullet points and headings not rendering** — Tailwind Preflight was stripping browser default styles (`list-style`, `font-size`, `font-weight`, link colors, table borders, spacing). Added comprehensive `!important` overrides to restore all defaults inside `.markdown-body`. (`5f12690`, `e0b90f2`)
- **Update notification too intrusive** — Removed auto-open Settings on update detection. Now only shows a system notification; user opens Settings manually when ready. (`304263a`)

### 📝 Prompt Improvements

- **Chain of Thought restructuring** — All 6 prompts (normal + beast × 3 modes) restructured with explicit thinking chains: ANALYZE → ERROR CORRECTION → REORGANIZE → OUTPUT. Organized into clear sections (`# ROLE`, `# THINKING CHAIN`, `# STRUCTURE`, `# FORMATTING`, `# CONSTRAINTS`). ~33% token reduction through deduplication. (`35704e1`)

## [0.5.0] - 2026-03-27

### ✨ Enhancements

- **Cancel in-flight LLM requests** — Click the spinning icon to cancel an ongoing request. Initial spinning state shows a red ✕ on hover; refresh button shows a red ■ stop square on hover. Backend uses `tokio::select!` with `CancellationToken` for immediate abort. (`0ece5da`, `845eee2`, `46115db`)
- **GitHub-flavored Markdown rendering** — Replaced Tailwind `prose` with `github-markdown-css` for consistent rendering of inline code, code blocks, tables, and all GFM features — matching GitHub's look and feel. (`382c628`)
- **Refresh spinner styling** — Spinner uses Copilot blue color with light blue background instead of plain gray. Hover transitions to red stop square on red background. (`8793124`)

### 📝 Prompt Improvements

- **No-answer rule enforced in all modes** — All 6 prompts now explicitly declare role boundaries (TRANSLATOR / POLISHER / REWRITER) and refuse to answer questions, provide solutions, or add opinions. (`c5e6201`)
- **Adaptive structure scaling** — Output structure scales with input length: short text stays simple, longer text gets headings, lists, and paragraph breaks. (`a06df77`)
- **Hierarchical structure with symmetry** — Prompts now instruct the LLM to use nested lists for parent-child relationships and tables for symmetric/parallel content (pros/cons, comparisons). (`ca2dbb8`)

## [0.4.0] - 2026-03-27

### ✨ Enhancements

- **Tray icon status indicator** — Tray icon now shows a colored dot in the bottom-right corner: green when active, red when disabled. Tooltip also updates to reflect current state. (`e60267e`)
- **Colored tray menu icons** — All tray menu items now have custom-drawn colored icons (blue info circle, gray gear, amber pause/green play, green refresh arrow, red X) instead of black-and-white emoji. (`e530b6b`, `eb42028`)
- **Tray Disable/Enable toggle fixed** — Clicking Disable now properly switches the menu label to Enable (and vice versa). Icon shape and color update dynamically. (`c56be6c`)
- **Version info in tray menu** — Version number shown at the top of the tray menu; clicking it opens the GitHub Release page. (`c56be6c`)
- **Update notification auto-opens Settings** — When an update is detected on startup, the Settings window automatically opens and activates so the user immediately sees the update banner. (`fcf214e`)

### 📝 Prompt Improvements

- **Full Markdown formatting** — All prompts now encourage the use of bold, italic, code blocks, blockquotes, tables, headings, ASCII diagrams, colored inline HTML, and emoji for richer, more expressive output. (`8af150c`, `c301cdf`)
- **Error correction in all modes** — All prompts (normal + beast) now silently fix typos, misspellings, wrong product names, and incorrect terminology. (`131f121`)
- **Beast mode: stronger rewriting** — Beast mode additionally replaces weak or inappropriate examples with better, more illustrative ones. (`1c3ba8c`)

## [0.3.0] - 2026-03-27

### ✨ Enhancements

- **Split Replace button — rendered text vs markdown mode** — Replace button now has a dropdown to switch between two modes: *Rendered text* (default) pastes rich HTML via `CF_HTML` clipboard format for Teams/Outlook; *Markdown* pastes plain text for GitHub editors. The choice is persisted across sessions. (`a1f03a7`)
- **Mode indicator icon on Replace button** — Replace button shows a contextual icon: formatted-lines icon for rendered mode, "MD" badge for markdown mode. Icons use the same stroke-based line art style as Copy and Refresh. (`3c6f405`)
- **Copy follows Replace mode** — Copy now respects the replace mode setting: in rendered mode it copies rich HTML (`CF_HTML` + plain text fallback); in markdown mode it copies plain Markdown text. (`66643b9`)

## [0.2.2] - 2026-03-27

### ✨ Enhancements

- **Automatic update check on startup** — The app now checks for updates 10 seconds after launch and sends a Windows system notification if a new version is available. No need to manually open Settings to discover updates. (`542dc4c`)

## [0.2.1] - 2026-03-27

### 🐛 Bug Fixes

- **Version display hardcoded** — Settings footer now reads the version dynamically from Tauri `getVersion()` API instead of a hardcoded string. (`b3b62c2`)

### ✨ Enhancements

- **Clickable version links** — Both the current version in the footer and the new version in the update banner are now clickable links that open the corresponding GitHub Release page. (`b3b62c2`, `1d2630f`)

## [0.2.0] - 2026-03-27

### 🐛 Bug Fixes

- **Auth status not refreshing after login** — Popup now checks auth on icon click instead of only at mount time. Users no longer need to restart the app after logging in via Settings. (`6c18ad0`, `c6032f5`)
- **Settings window not coming to front** — When Settings is behind other windows (e.g., Teams), clicking the gear icon now reliably brings it to the foreground using Win32 `BringWindowToTop` + `SetForegroundWindow`. (`f2b6015`)
- **Popup triggering on read-only content** — Popup now only appears on editable elements (input, textarea, contenteditable). Selecting text on read-only web pages (GitHub, docs) no longer triggers the popup. (`d5d4e0e`, `1ca1fd8`)
- **Incompatible models causing HTTP 400 errors** — Models that don't support the `/chat/completions` endpoint (e.g., `gpt-5.4-mini`) are now filtered out of the model picker. (`9bd1913`)

### ✨ Enhancements

- **Model picker grouped by vendor** — Models are now organized under vendor headers (Anthropic, Google, OpenAI, etc.) with alphabetical sorting within each group. (`122d344`)
- **Model category indicator** — Powerful models (Opus, GPT-5.2-Codex, etc.) are marked with a lightning icon in the picker to indicate higher resource usage. (`d415540`)
- **Chat completions hint** — Added a note below the model dropdown explaining that only chat-completions-compatible models are listed. (`ff22b4f`)
- **Beast mode default ON** — Beast mode is now enabled by default for new installations. (`5adbad4`)
- **Quick access to Settings** — Model name and beast mode icon in the Popup bottom bar are now clickable buttons that open Settings. (`5adbad4`)

### 🔧 Internal

- Optimized CI build: limited bundle to NSIS, improved caching. (`170c5cd`)
- CI now extracts changelog into GitHub Release body automatically.

## [0.1.0] - 2026-03-26

- Initial release.
