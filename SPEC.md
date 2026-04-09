# Copilot Rewrite — SPEC.md

> **Purpose:** This is the living design spec for Copilot Rewrite. Every design decision and directive from Miller is recorded here. AI agents MUST read this before making changes.

**Last Updated:** 2026-04-09

---

## 1. Product Overview

System-level text translation and polishing tool for Windows. Works across all applications (Teams, Outlook, Edge, GitHub, etc.) via Windows UI Automation.

- **Tech Stack:** Tauri 2.0 (Rust + WebView2), React + Tailwind, Copilot API
- **Project:** `C:\Users\wangmi\projects\copilot-rewrite`

---

## 2. Two Modes

### ✍️ Write Mode (Input Elements)
- User selects text in an **input field** (`<input>`, `<textarea>`, `contentEditable`)
- Actions: **Polish**, **Translate**, **Translate + Polish**
- Result replaces the original text in-place (clipboard + Ctrl+V)

### 📖 Read Mode (Non-Input Elements)
- User selects text on a **webpage or document** (`<p>`, `<span>`, etc.)
- Actions: **Translate**, **Summarize**
- Result displayed in popup (no replacement — read-only context)
- Enabled/disabled via settings toggle (`read_mode_enabled`)

---

## 3. Popup Lifecycle — CORE DESIGN RULES

> These rules are **absolute**. Do not add exceptions or state-dependent branching.

### 3.1 Trigger: Show Popup

**Primary (mouse-driven, ~90% of cases):**
1. User presses mouse button (mousedown) — potential selection start
2. User drags to select text
3. User releases mouse button (mouseup) — selection complete
4. **On mouseup → execute ONE UIA check → if selection exists → show popup icon**
5. **Popup MUST NOT appear during drag** (while mouse button is held)

**Fallback (keyboard selection, ~10%):**
- Low-frequency UIA poll (every 500ms–1s) to catch Ctrl+A, Shift+Arrow, etc.
- Debounce still applies for fallback path

**Key principle:** Mouse events drive the popup, not UIA polling. UIA is only called reactively (on mouseup or at low frequency as fallback).

### 3.2 Popup States

```
[No Selection] → mouseup detected → UIA check → [Icon Visible]
[Icon Visible] → user clicks icon → [Spinning] (API in-flight)
[Spinning] → API returns → [Expanded] (showing result)
[Expanded] → user interacts (Replace/Copy/Cancel)
```

### 3.3 Dismiss: Hide Popup — THE UNCONDITIONAL RULE

> **Selection gone = Popup gone.** No exceptions. No state checks.

When text selection is cleared (UIA returns no selection):
- Hide the popup **immediately**
- Cancel any in-flight API request
- Reset ALL state (preview_visible, selection, generation counter)
- **This applies regardless of popup state** (icon, spinning, or expanded)

**Do NOT gate dismiss logic on popup state.** The dismiss behavior is at the popup level, not tied to whether we're in icon/spinning/expanded state.

### 3.4 Dismiss Triggers

| Trigger | Mechanism | Applies To |
|---------|-----------|------------|
| Selection cleared | UIA returns no text | ALL states |
| Mouse click elsewhere | mousedown detection | Icon state (Read Mode) |
| Foreground window changed | HWND comparison | ALL states |
| User clicks Cancel | Frontend button | Expanded state |
| Escape key | Frontend handler | Expanded state |

### 3.5 Popup Positioning

- **Primary:** Mouse cursor position (`GetCursorPos`)
- Selection bounding rect not available in Chromium apps
- Screen boundary detection to prevent clipping
- **MUST NOT steal focus** from source application (`WS_EX_NOACTIVATE`)

---

## 4. Selection Detection Architecture

### 4.1 Current: UIA Polling (to be refactored)
- Polls every 100ms via `UiaEngine.get_selected_text_any()`
- High CPU usage, unnecessary calls when user is not selecting

### 4.2 Target: Event-Driven (mouseup + low-freq fallback)

```
┌─────────────────────────────────────────────┐
│           Selection Detection               │
│                                             │
│  [Mouse Hook / GetAsyncKeyState]            │
│    mousedown → track drag state             │
│    mouseup   → trigger ONE UIA check        │
│                                             │
│  [Low-Frequency Fallback Poll]              │
│    Every 500ms-1s → catch keyboard          │
│    selections (Ctrl+A, Shift+Arrow)         │
│                                             │
│  [UIA TextSelectionChanged Event]           │
│    Optional optimization — fires on         │
│    selection change, but unreliable          │
│    (event storms from terminals)            │
└─────────────────────────────────────────────┘
```

**Benefits:**
- CPU usage drops from ~10 UIA calls/sec to ~0 (idle) + 1 per mouseup
- Popup appears faster (no debounce needed for mouseup path)
- Cleaner architecture — events drive behavior, not polling

---

## 5. Text Replacement

- **Method:** Clipboard save → write result → simulate Ctrl+V → clipboard restore
- **Rendered mode:** Uses `CF_HTML` clipboard format for rich text (Teams, Outlook)
- **Markdown mode:** Plain text paste (GitHub, code editors)
- User can toggle between rendered/markdown in popup

---

## 6. LLM Backend

- **API:** GitHub Copilot API (via OAuth device flow)
- **Streaming:** Results stream progressively into popup
- **Beast Mode:** More aggressive rewriting (restructure, fix factual errors)
- **Error correction:** Silently fixes typos, wrong product names, incorrect terminology

---

## 7. Settings

- Native Language (mother tongue for output)
- Target Language (for translations)
- Read Mode toggle
- Beast Mode toggle
- Replace mode (Rendered vs Markdown)
- Auto-update check on startup
- App blacklist (future)

---

## 8. Known Platform Limitations

| Issue | Description | Workaround |
|-------|-------------|------------|
| UIA stale selection | `TextPattern.GetSelection()` takes 1-3s to return empty after deselection | Mouse-based dismiss bypasses this |
| UIA event storms | Terminal apps (WindowsTerminal) fire TextSelectionChanged every ~2ms | Atomic bool handler absorbs; ignore in logic |
| Chromium selection rect | Selection bounding rect unavailable in Chromium-based apps | Use mouse position for popup placement |
| Tauri window.hide() | Sometimes fails to actually hide the window | Win32 `ShowWindow(SW_HIDE)` + `IsWindowVisible` verification |
| Debug console window | `windows_subsystem = "windows"` only applies in release builds | Use `-WindowStyle Hidden` for dev testing |

---

## 9. App Compatibility

| Application | UIA Support | Method |
|-------------|:-----------:|--------|
| Microsoft Teams | ✅ | UIA TextPattern |
| Microsoft Outlook | ✅ | UIA TextPattern |
| Microsoft Edge / Chrome | ✅ | UIA TextPattern |
| VS Code | ❌ | Clipboard fallback |
| Feishu (飞书) | ❌ | Clipboard fallback |
| Notepad / WordPad | ✅ | UIA TextPattern |

---

## 10. Design Principles (from Miller)

1. **Selection gone = Popup gone** — Dismiss is unconditional, never gated on popup state
2. **Mouse events drive popup** — Don't rely on polling; use mouseup as the primary trigger
3. **One popup, one behavior** — The popup is a single entity; its dismiss/show behavior should not branch based on internal state (icon vs spinning vs expanded)
4. **Minimize UIA calls** — Only call UIA when there's a reason (mouseup, keyboard fallback), not continuously

---

## Changelog

| Date | Change | Directive |
|------|--------|-----------|
| 2025-03-25 | Initial spec created | Product vision, UIA validation, tech stack |
| 2026-03-28 | Read Mode added | Separate mode for non-input text (translate + summarize) |
| 2026-04-09 | Unconditional dismiss rule | "Selection cleared = hide popup regardless of state" |
| 2026-04-09 | Mouse-driven popup trigger | "Use mouseup to trigger popup, not polling" |
| 2026-04-09 | Suppress popup during drag | "Popup must not appear while mouse is held down" |
