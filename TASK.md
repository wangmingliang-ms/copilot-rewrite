# Task: Implement Phase 1 MVP of Copilot Rewrite

Read the full spec at: C:\Users\wangmi\.openclaw\workspace-ideas\copilotwrite-spec.md

## What to Build

Create a Tauri 2.0 app (Rust + React + Tailwind CSS) in this directory with these features:

### 1. Tauri app skeleton + system tray
- Background app with tray icon
- Enable/disable toggle in tray menu
- Minimize to tray on close

### 2. UIA polling engine
- Use Rust `windows` crate to access IUIAutomation COM interfaces
- Poll every 100ms to detect text selection via TextPattern.GetSelection()
- Works in Teams, Outlook, Edge (verified)
- Debounce: wait 200ms after selection stabilizes before showing toolbar

### 3. Mouse position tracking
- Use GetCursorPos Win32 API for popup positioning
- This is the PRIMARY positioning method (selection bounding rects are NOT available in Chromium apps)

### 4. Floating toolbar
- Non-focus-stealing overlay window using WS_EX_NOACTIVATE
- Appears near mouse cursor when text is selected (offset +8px right, +16px below)
- Three buttons: Translate, Polish, Both (Translate + Polish)
- Screen boundary detection to prevent clipping
- Auto-dismiss when user clicks elsewhere or presses Escape
- Must NOT steal focus from the source application

### 5. Copilot API integration
- Call GitHub Copilot API for translation/polishing
- Non-streaming for MVP
- Auto-detect source language
- Translate to user-configured target language (default: English)
- Pre-configured system prompts for translate / polish / both modes

### 6. Preview popup
- Shows original text and processed result
- Three action buttons: Replace, Copy, Cancel
- Also must not steal focus

### 7. Text replacement
- Save current clipboard content
- Write result to clipboard
- Simulate Ctrl+V keystroke to paste into the focused application
- Restore original clipboard content

### 8. Auto-start on Windows login
- Register in Windows startup registry

## Tech Stack
- Tauri 2.0 (Rust backend + WebView2 frontend)
- React + Tailwind CSS for UI components
- Rust `windows` crate for UIA and Win32 APIs
- Rust `clipboard-win` crate for clipboard management

## Please create ALL necessary files:
- Cargo.toml with all Rust dependencies
- package.json with React/Tailwind dependencies
- tauri.conf.json configuration
- All Rust source files (main.rs, lib.rs, modules for UIA, clipboard, etc.)
- All React components (Toolbar, Preview, App)
- Tailwind config
- index.html
