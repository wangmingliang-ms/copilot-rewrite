import { useState, useCallback, useEffect, type FC } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { SelectionInfo, ProcessResponse } from "../hooks/useSelection";

type PopupState = "icon" | "spinning" | "expanded" | "error";

interface PopupProps {
  selection: SelectionInfo | null;
  authStatus: { logged_in: boolean; username: string | null };
}

const Popup: FC<PopupProps> = ({ selection, authStatus }) => {
  const [state, setState] = useState<PopupState>("icon");
  const [result, setResult] = useState<ProcessResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Listen for backend events
  useEffect(() => {
    const unResult = listen<ProcessResponse>("show-preview-result", (event) => {
      setResult(event.payload);
      setError(null);
      setState("expanded");
    });

    const unLoading = listen("show-preview-loading", () => {
      setState("spinning");
      setError(null);
      setResult(null);
    });

    const unError = listen<string>("show-preview-error", (event) => {
      setError(event.payload);
      setState("error");
    });

    // Reset to icon state when popup is hidden and re-shown
    const unSelection = listen("selection-detected", () => {
      setState("icon");
      setResult(null);
      setError(null);
    });

    return () => {
      unResult.then((f) => f());
      unLoading.then((f) => f());
      unError.then((f) => f());
      unSelection.then((f) => f());
    };
  }, []);

  // Dismiss on Escape
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") handleDismiss();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  // Auto-dismiss on blur (when expanded)
  useEffect(() => {
    if (state !== "expanded" && state !== "error") return;
    const handleBlur = () => {
      setTimeout(async () => {
        if (!document.hasFocus()) {
          handleDismiss();
        }
      }, 100);
    };
    window.addEventListener("blur", handleBlur);
    return () => window.removeEventListener("blur", handleBlur);
  }, [state]);

  const handleIconClick = useCallback(async () => {
    if (!authStatus.logged_in) {
      try {
        await invoke("open_settings");
      } catch {
        setError("Please login via tray → Settings");
        setState("error");
      }
      return;
    }
    if (!selection) return;

    setState("spinning");
    try {
      await invoke("process_and_show_preview", {
        request: { text: selection.text, action: "TranslateAndPolish" },
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setState("error");
    }
  }, [authStatus, selection]);

  const handleReplace = useCallback(async () => {
    if (!result) return;
    try {
      await invoke("replace_text", { text: result.result });
      await invoke("dismiss_popup");
      resetState();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [result]);

  const handleCopy = useCallback(async () => {
    if (!result) return;
    try {
      await invoke("copy_to_clipboard", { text: result.result });
      await invoke("dismiss_popup");
      resetState();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [result]);

  const handleDismiss = useCallback(async () => {
    try {
      await invoke("dismiss_popup");
    } catch {}
    resetState();
  }, []);

  const resetState = () => {
    setState("icon");
    setResult(null);
    setError(null);
  };

  // ── Icon state (48×48) ──
  if (state === "icon") {
    return (
      <div className="w-screen h-screen flex items-center justify-center"
        style={{
          background: "rgba(255,255,255,0.95)",
          borderRadius: "24px",
          border: "1px solid #e0e0e0",
          boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
        }}
      >
        <button
          onPointerDown={(e) => { e.preventDefault(); handleIconClick(); }}
          className="w-full h-full flex items-center justify-center"
          style={{ cursor: "pointer", borderRadius: "24px" }}
          title="Translate & Polish"
        >
          <svg className="w-7 h-7" viewBox="0 0 24 24" fill="none" stroke="#0078D4" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M5 8l4-4 4 4" />
            <path d="M9 4v8" />
            <path d="M12 20l3-6 3 6" />
            <path d="M13.5 17h3" />
            <path d="M2 16h4" />
            <path d="M6 12c0 3-2.5 4-2.5 4" />
            <path d="M6 12c0 0 2 1 4 1" />
          </svg>
        </button>
      </div>
    );
  }

  // ── Spinning state (48×48 with spinner) ──
  if (state === "spinning") {
    return (
      <div className="w-screen h-screen flex items-center justify-center"
        style={{
          background: "rgba(255,255,255,0.95)",
          borderRadius: "24px",
          border: "1px solid #e0e0e0",
          boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
        }}
      >
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-blue-500 border-t-transparent" />
      </div>
    );
  }

  // ── Error state ──
  if (state === "error") {
    return (
      <div className="flex flex-col rounded-xl shadow-popup"
        style={{
          background: "rgba(255,255,255,0.98)",
          border: "1px solid #e0e0e0",
          boxShadow: "0 4px 16px rgba(0,0,0,0.15)",
        }}
      >
        <div className="px-4 py-3">
          <p className="text-xs text-red-500">{error}</p>
        </div>
        <div className="flex justify-end border-t border-gray-100 px-3 py-2">
          <button
            onClick={handleDismiss}
            className="rounded-md px-3 py-1.5 text-xs font-medium text-gray-500 hover:bg-gray-100"
          >
            Close
          </button>
        </div>
      </div>
    );
  }

  // ── Expanded state (auto-sized with result) ──
  return (
    <div className="flex flex-col rounded-xl"
      style={{
        background: "rgba(255,255,255,0.98)",
        border: "1px solid #e0e0e0",
        boxShadow: "0 4px 16px rgba(0,0,0,0.15)",
      }}
    >
      <div className="px-4 py-3 overflow-auto" style={{ maxHeight: "340px" }}>
        <div className="text-sm leading-relaxed text-gray-800 whitespace-pre-wrap">
          {result?.result}
        </div>
      </div>
      <div className="flex items-center justify-end gap-1.5 border-t border-gray-100 px-3 py-2">
        <button
          onClick={handleDismiss}
          className="rounded-md px-3 py-1.5 text-xs font-medium text-gray-400 hover:bg-gray-100 hover:text-gray-600"
        >
          ✕
        </button>
        <button
          onClick={handleCopy}
          className="rounded-md px-3 py-1.5 text-xs font-medium text-gray-600 hover:bg-gray-100"
        >
          📋
        </button>
        <button
          onClick={handleReplace}
          className="rounded-md bg-copilot-blue px-3 py-1.5 text-xs font-medium text-white hover:bg-copilot-blue-hover"
        >
          Replace
        </button>
      </div>
    </div>
  );
};

export default Popup;
