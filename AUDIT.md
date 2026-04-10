# Project Audit — Bugs, Performance, and Efficiency Findings

**Date:** 2026-04-10
**Scope:** All Rust source files in `src-tauri/src/`

---

## 🔴 Critical / High Severity

### 1. SSE streaming reads entire body into memory, defeating streaming purpose
**File:** `copilot/client.rs` lines 586-624 (and 702-728)
**Category:** Latency
**Status:** TODO

Both `process()` and `process_read_mode()` set `stream: true` but call `response.text().await` — downloading the entire body before parsing SSE. User sees zero progress until LLM finishes. Time-to-first-token = total generation time instead of ~200ms.

### 2. Clipboard content not saved/restored during replace — user loses clipboard
**File:** `replacement/engine.rs` lines 34-110
**Category:** Data loss
**Status:** ✅ FIXED (commit 54fb7b8)

`ClipboardGuard` exists but is never used in `replace_selected_text()`. Every Replace destroys the user's clipboard.

### 3. COM initialized with COINIT_MULTITHREADED but UIA works best with STA
**File:** `selection/uia.rs` line 231
**Category:** Potential crashes
**Status:** ✅ FIXED (commit 75dc02b)

MTA allows COM callbacks from any thread. UIA interacts with UI elements and works best in STA.

### 4. `get_copilot_token` 401 retry: cache cleared but no retry
**File:** `copilot/client.rs` lines 574-583
**Category:** Functional bug
**Status:** ✅ FIXED (commit 0ffb55d)

On 401, token cache is cleared but error is returned immediately. User must manually retry.

---

## 🟠 Medium Severity

### 5. `process()` and `process_read_mode()` are 90% duplicated
**File:** `copilot/client.rs`
**Category:** Maintainability
**Status:** ✅ FIXED (commit 0ffb55d)

SSE parsing, error handling, HTTP construction are copy-pasted. Only system prompt differs.

### 6. Logging opens and flushes log file on EVERY log line
**File:** `lib.rs` lines 838-849
**Category:** Performance
**Status:** ✅ FIXED (commit 2a52302)

Every INFO+ log line does file open/write/flush/close. Hundreds of cycles per second on busy sessions.

### 7. SafeArray not properly guarded on error path
**File:** `selection/uia.rs` lines 352-393
**Category:** Resource leak
**Status:** ✅ FIXED (commit 5218eb1)

Between `SafeArrayAccessData` and `SafeArrayUnaccessData`, if bounds are garbage, slice is invalid.

### 8. `replace_text` re-enables monitoring after only 300ms
**File:** `lib.rs` lines 691-696
**Category:** Race condition
**Status:** ✅ FIXED (commit 4b3a39c)

Slow apps (Outlook) may not process paste in 300ms. Popup reappears immediately after Replace.

### 9. `hide_popup` calls `shrink_popup` unnecessarily
**File:** `overlay/mod.rs` lines 316-335
**Category:** Performance
**Status:** ✅ FIXED (commit 7826591)

Hidden window doesn't need resize. Defer to next `show_popup_icon`.

### 10. Overlay functions repeatedly look up popup window by name
**File:** `overlay/mod.rs`
**Category:** Performance
**Status:** ✅ FIXED — cached `WebviewWindow` in static `POPUP_WINDOW`, set once in `setup_popup_window()`. All overlay functions now use `get_popup()` helper.

### 11. OAuth creates new `reqwest::Client` per call instead of reusing
**File:** `lib.rs` lines 438, 456
**Category:** Latency
**Status:** ✅ FIXED (commit 30d06c7)

New client = new connection pool. Should reuse `CopilotClient.http`.

### 12. `shrink_popup` uses approximate DPI calculation
**File:** `overlay/mod.rs` lines 303-306
**Category:** Correctness
**Status:** ✅ FIXED (commit ec10abc)

`px * 1.5` guess is only correct at 150% DPI. Wrong on other scales or multi-monitor.

---

## 🟡 Low Severity

### 13. `estimate_height` double-scans string
**File:** `overlay/mod.rs` line 392
**Category:** Performance
**Status:** ✅ FIXED — rewritten as single-pass loop counting chars and newlines simultaneously.

### 14. `build_cf_html` header offset relies on placeholder length invariant
**File:** `clipboard/manager.rs` lines 62-82
**Category:** Correctness
**Status:** ✅ FIXED — added safety invariant doc comment explaining the 10-char placeholder ↔ `{:010}` format dependency.

### 15. `debug_log` in replacement engine opens file per call
**File:** `replacement/engine.rs` lines 19-31
**Category:** Performance
**Status:** ✅ FIXED — cached file handle in `thread_local!` static, opened once per thread on first use.

### 16. `extract_translated` doesn't handle Unicode escapes
**File:** `overlay/mod.rs` lines 401-433
**Category:** Cosmetic
**Status:** ✅ FIXED — function removed entirely (dead code after switch from JSON to separator format for Read Mode).

### 17. `image` crate may be unused
**File:** `Cargo.toml` line 29
**Category:** Build time
**Status:** ✅ FIXED (commit 05fa2fa)

### 18. `once_cell` may be replaceable with std
**File:** `Cargo.toml` line 31
**Category:** Dependencies
**Status:** ✅ FIXED — removed from Cargo.toml (not used in source code, only pulled by transitive dependencies).

### 19. `open_settings` HWND cast inconsistency
**File:** `lib.rs` line 593
**Category:** Correctness
**Status:** ✅ FIXED (commit 05fa2fa)

### 20. No TCP keepalive on HTTP client
**File:** `copilot/client.rs` line 390-393
**Category:** Latency
**Status:** ✅ FIXED — added `.tcp_keepalive(Duration::from_secs(30))` to the `reqwest::Client` builder.
