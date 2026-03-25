import { useState, useCallback, useEffect, useRef } from "react";
import Toolbar from "./Toolbar";
import Preview from "./Preview";
import SettingsPanel from "./SettingsPanel";
import { useSelection, ProcessResponse } from "../hooks/useSelection";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface AuthStatus {
  logged_in: boolean;
  username: string | null;
}

function App() {
  const [processResult, setProcessResult] = useState<ProcessResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [authStatus, setAuthStatus] = useState<AuthStatus>({ logged_in: false, username: null });

  const { selection } = useSelection();
  const prevSelectionRef = useRef<string | null>(null);

  // Route based on URL hash
  const hash = window.location.hash;

  // ── Settings view ──
  if (hash === "#/settings") {
    return <div className="settings-view"><SettingsPanel /></div>;
  }

  // ── Preview view ── listens for Tauri events from backend
  if (hash === "#/preview") {
    return <div className="preview-view"><PreviewWindow /></div>;
  }

  // ── Toolbar view ── (default)

  // Check auth status on mount
  useEffect(() => {
    invoke<AuthStatus>("get_auth_status").then(setAuthStatus).catch(() => {});
  }, []);

  // Clear error state when new text is selected
  useEffect(() => {
    if (selection?.text && selection.text !== prevSelectionRef.current) {
      prevSelectionRef.current = selection.text;
      if (error) setError(null);
      if (processResult) setProcessResult(null);
    }
  }, [selection?.text, error, processResult]);

  const handleAction = useCallback(async () => {
    console.log("[App] handleAction called, authStatus:", JSON.stringify(authStatus));
    console.log("[App] selection:", selection?.text?.substring(0, 30));
    
    if (!authStatus.logged_in) {
      console.log("[App] Not logged in, opening settings");
      try {
        await invoke("open_settings");
      } catch (e) {
        console.error("[App] open_settings failed:", e);
        setError("Please login via tray → Settings");
      }
      return;
    }

    if (!selection) {
      console.log("[App] No selection, returning");
      return;
    }
    
    console.log("[App] Calling process_and_show_preview with text:", selection.text.substring(0, 50));
    // Immediately hide toolbar — don't show loading spinner here, preview window handles it
    setError(null);
    try {
      await invoke("dismiss_toolbar");
      await invoke("process_and_show_preview", {
        request: { text: selection.text, action: "TranslateAndPolish" },
      });
      console.log("[App] process_and_show_preview succeeded");
    } catch (err) {
      console.error("[App] process_and_show_preview failed:", err);
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [authStatus, selection]);

  const handleDismissToolbar = useCallback(async () => {
    await invoke("dismiss_toolbar");
    setError(null);
  }, []);

  return (
    <div className="toolbar-view">
      <Toolbar
        selection={selection}
        loading={false}
        error={error}
        onAction={handleAction}
        onDismiss={handleDismissToolbar}
      />
    </div>
  );
}

// ── Preview Window Component ──
// Runs in the preview Tauri window, listens for events from backend

function PreviewWindow() {
  const [result, setResult] = useState<ProcessResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // Listen for show-preview event from Rust backend
    const unlistenResult = listen<ProcessResponse>("show-preview-result", (event) => {
      setResult(event.payload);
      setLoading(false);
      setError(null);
    });

    const unlistenLoading = listen("show-preview-loading", () => {
      setLoading(true);
      setError(null);
      setResult(null);
    });

    const unlistenError = listen<string>("show-preview-error", (event) => {
      setError(event.payload);
      setLoading(false);
    });

    return () => {
      unlistenResult.then(f => f());
      unlistenLoading.then(f => f());
      unlistenError.then(f => f());
    };
  }, []);

  const handleReplace = useCallback(async () => {
    if (!result) return;
    try {
      console.log("[Replace] Starting replace_text invoke, text length:", result.result.length);
      await invoke("replace_text", { text: result.result });
      console.log("[Replace] replace_text invoke succeeded");
      await invoke("dismiss_preview");
      console.log("[Replace] dismiss_preview invoke succeeded");
      setResult(null);
    } catch (err) {
      console.error("[Replace] invoke FAILED:", err);
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [result]);

  const handleCopy = useCallback(async () => {
    if (!result) return;
    try {
      await invoke("copy_to_clipboard", { text: result.result });
      await invoke("dismiss_preview");
      setResult(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [result]);

  const handleCancel = useCallback(async () => {
    await invoke("dismiss_preview");
    setResult(null);
    setError(null);
  }, []);

  return (
    <Preview
      result={result}
      loading={loading}
      error={error}
      onReplace={handleReplace}
      onCopy={handleCopy}
      onCancel={handleCancel}
    />
  );
}

export default App;
