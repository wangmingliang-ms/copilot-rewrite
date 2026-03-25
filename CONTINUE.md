Continue implementing the Copilot Rewrite project. The previous run created the project structure but was interrupted before finishing. 

The following directories are EMPTY and need files created:
- src-tauri/src/copilot/ (needs mod.rs and client.rs - Copilot API client)
- src-tauri/src/overlay/ (needs mod.rs - overlay window management)
- src-tauri/src/tray/ (needs mod.rs - system tray setup)
- src-tauri/src/autostart/ (needs mod.rs - Windows autostart registration)
- src/components/ (needs Toolbar.tsx, Preview.tsx, App.tsx - React components)
- src/hooks/ (needs useSelection.ts - hook for selection events)
- src/styles/ (needs index.css - Tailwind CSS)

Also need:
- src/main.tsx (React entry point)

Check the existing files (lib.rs, main.rs, Cargo.toml, etc.) to understand the module structure already defined, and create the missing files to match.

Key requirements:
- Copilot API client: POST to https://api.githubcopilot.com/chat/completions with system prompts for translate/polish/both
- Overlay: manage floating toolbar and preview windows (WS_EX_NOACTIVATE)
- Tray: system tray icon with Enable/Disable/Quit menu
- Autostart: register in HKCU\Software\Microsoft\Windows\CurrentVersion\Run
- React Toolbar: 3 buttons (Translate, Polish, Both), appears via Tauri events
- React Preview: shows original + result, Replace/Copy/Cancel buttons
