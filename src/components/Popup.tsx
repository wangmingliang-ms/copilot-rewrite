import { useState, useCallback, useEffect, useMemo, type FC } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { marked } from "marked";
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
  const [showOriginal, setShowOriginal] = useState(false);

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

  // Parse result — LLM returns JSON {"reorganized": "...", "translated": "..."}
  const { reorganized, translated } = useMemo(() => {
    if (!result?.result) return { reorganized: "", translated: "" };
    const text = result.result.trim();
    try {
      // Strip markdown code fences if LLM wraps in ```json ... ```
      const cleaned = text.replace(/^```(?:json)?\s*\n?/i, "").replace(/\n?```\s*$/, "");
      const parsed = JSON.parse(cleaned);
      if (parsed.reorganized && parsed.translated) {
        return {
          reorganized: String(parsed.reorganized),
          translated: String(parsed.translated),
        };
      }
    } catch {
      // JSON parse failed — fallback: try "---" divider
      const dividerMatch = text.match(/\n---\n/);
      if (dividerMatch && dividerMatch.index !== undefined) {
        return {
          reorganized: text.slice(0, dividerMatch.index).trim(),
          translated: text.slice(dividerMatch.index + dividerMatch[0].length).trim(),
        };
      }
    }
    // No structure found — treat entire text as translated
    return { reorganized: "", translated: text };
  }, [result?.result]);

  // Render markdown
  const reorganizedHtml = useMemo(() => {
    if (!reorganized) return "";
    marked.setOptions({ breaks: true, gfm: true });
    return marked.parse(reorganized) as string;
  }, [reorganized]);

  const translatedHtml = useMemo(() => {
    if (!translated) return "";
    marked.setOptions({ breaks: true, gfm: true });
    return marked.parse(translated) as string;
  }, [translated]);

  // The text to use for Replace/Copy — always the translated version
  const outputText = translated || result?.result || "";

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

  const handleRefresh = useCallback(async () => {
    if (!selection) return;
    setState("spinning");
    setResult(null);
    setError(null);
    try {
      await invoke("process_and_show_preview", {
        request: { text: selection.text, action: "TranslateAndPolish" },
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setState("error");
    }
  }, [selection]);

  const handleReplace = useCallback(async () => {
    if (!outputText) return;
    try {
      await invoke("replace_text", { text: outputText });
      await invoke("dismiss_popup");
      resetState();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [outputText]);

  const handleCopy = useCallback(async () => {
    if (!outputText) return;
    try {
      await invoke("copy_to_clipboard", { text: outputText });
      await invoke("dismiss_popup");
      resetState();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [outputText]);

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
    setShowOriginal(false);
  };

  // ── Icon state (48×48) ──
  if (state === "icon") {
    return (
      <div className="w-screen h-screen flex items-center justify-center" style={{ padding: "20px", background: "transparent" }}>
        <div className="w-full h-full flex items-center justify-center"
          style={{
            background: "linear-gradient(135deg, #fff 0%, #f0f4ff 100%)",
            borderRadius: "14px",
            border: "1px solid rgba(0,120,212,0.15)",
            boxShadow: "0 4px 16px rgba(0,120,212,0.12), 0 1px 3px rgba(0,0,0,0.08)",
          }}
        >
          <button
            onPointerDown={(e) => { e.preventDefault(); handleIconClick(); }}
            className="w-full h-full flex items-center justify-center group"
            style={{ cursor: "pointer", borderRadius: "14px" }}
            title="Translate & Polish"
          >
            <svg className="w-6 h-6 transition-transform group-hover:scale-110" viewBox="0 0 24 24" fill="none" stroke="#0078D4" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
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
      </div>
    );
  }

  // ── Spinning state (48×48 with spinner) ──
  if (state === "spinning") {
    return (
      <div className="w-screen h-screen flex items-center justify-center" style={{ padding: "20px", background: "transparent" }}>
        <div className="w-full h-full flex items-center justify-center"
          style={{
            background: "linear-gradient(135deg, #fff 0%, #f0f4ff 100%)",
            borderRadius: "14px",
            border: "1px solid rgba(0,120,212,0.15)",
            boxShadow: "0 4px 16px rgba(0,120,212,0.12), 0 1px 3px rgba(0,0,0,0.08)",
          }}
        >
          <div className="h-5 w-5 animate-spin rounded-full border-2 border-copilot-blue border-t-transparent" />
        </div>
      </div>
    );
  }

  // ── Error state ──
  if (state === "error") {
    return (
      <div className="w-screen h-screen" style={{ padding: "20px", background: "transparent" }}>
        <div className="flex flex-col rounded-lg h-full overflow-hidden"
          style={{
            background: "#fff",
            border: "1px solid rgba(0,0,0,0.08)",
            boxShadow: "0 8px 32px rgba(0,0,0,0.12), 0 2px 8px rgba(0,0,0,0.06)",
          }}
        >
          <div className="px-5 py-4 flex items-start gap-2.5">
            <div className="flex-shrink-0 w-5 h-5 rounded-full bg-red-50 flex items-center justify-center mt-0.5">
              <svg className="w-3 h-3 text-red-500" viewBox="0 0 12 12" fill="currentColor">
                <path d="M6 0a6 6 0 100 12A6 6 0 006 0zm.75 9h-1.5V7.5h1.5V9zm0-3h-1.5V3h1.5v3z"/>
              </svg>
            </div>
            <p className="text-[13px] leading-[1.5] text-red-600">{error}</p>
          </div>
          <div className="flex justify-end border-t border-gray-100 px-3 py-2"
            style={{ background: "rgba(249,250,251,0.8)" }}
          >
            <button
              onClick={handleDismiss}
              className="rounded-lg px-3 py-1.5 text-xs font-medium text-gray-500 hover:bg-gray-200/60 transition-colors"
            >
              Close
            </button>
          </div>
        </div>
      </div>
    );
  }

  // ── Expanded state (auto-sized with result) ──
  return (
    <div className="w-screen h-screen" style={{ padding: "20px", background: "transparent" }}>
      <div className="flex flex-col rounded-lg overflow-hidden pt-1"
        style={{
          background: "#fff",
          border: "1px solid rgba(0,0,0,0.08)",
          boxShadow: "0 8px 32px rgba(0,0,0,0.12), 0 2px 8px rgba(0,0,0,0.06)",
        }}
      >
        {/* Translation — scrollable */}
        <div className="px-5 pt-5 pb-3">
          <div className="overflow-auto" style={{ maxHeight: "280px", userSelect: "text", WebkitUserSelect: "text" }}>
            <div
              className="text-[13.5px] leading-[1.7] text-gray-800 prose prose-sm max-w-none prose-p:my-1 prose-li:my-0.5 prose-ul:my-1.5 prose-ol:my-1.5 prose-headings:my-1.5 prose-strong:text-gray-900"
              dangerouslySetInnerHTML={{ __html: translatedHtml }}
            />
          </div>
        </div>

        {/* Reorganized — fixed below translation, above action bar */}
        {reorganizedHtml && (
          <div className="px-5 pb-2">
            <div className="pt-2 border-t border-gray-100">
              <button
                onClick={() => setShowOriginal(!showOriginal)}
                className="flex items-center gap-1 text-[11px] text-gray-400 hover:text-gray-600 transition-colors"
              >
                <svg
                  className={`w-3 h-3 transition-transform ${showOriginal ? "rotate-90" : ""}`}
                  viewBox="0 0 12 12" fill="currentColor"
                >
                  <path d="M4.5 2l5 4-5 4V2z" />
                </svg>
                <span className="font-medium tracking-wide uppercase">Original (reorganized)</span>
              </button>
              {showOriginal && (
                <div className="mt-2 overflow-auto text-[12px] leading-[1.55] text-gray-400 prose prose-sm max-w-none prose-p:my-0.5 prose-li:my-0.5 prose-ul:my-1 prose-ol:my-1"
                  style={{ maxHeight: "120px" }}
                >
                  <div dangerouslySetInnerHTML={{ __html: reorganizedHtml }} />
                </div>
              )}
            </div>
          </div>
        )}

        {/* Action bar */}
        <div className="flex items-center justify-between border-t border-gray-100 px-3 py-2"
          style={{ background: "rgba(249,250,251,0.8)" }}
        >
          <button
            onClick={handleDismiss}
            className="flex items-center justify-center w-7 h-7 rounded-lg text-gray-400 hover:bg-gray-200/60 hover:text-gray-600 transition-colors"
            title="Dismiss"
          >
            <svg className="w-3.5 h-3.5" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
              <path d="M1 1l12 12M13 1L1 13" />
            </svg>
          </button>
          <div className="flex items-center gap-1">
            <button
              onClick={handleRefresh}
              className="flex items-center justify-center w-7 h-7 rounded-lg text-gray-500 hover:bg-gray-200/60 hover:text-gray-700 transition-colors"
              title="Regenerate"
            >
              <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                <path d="M2.5 8a5.5 5.5 0 0 1 9.3-4" />
                <path d="M13.5 8a5.5 5.5 0 0 1-9.3 4" />
                <path d="M11.5 1.5v3h3" />
                <path d="M4.5 14.5v-3h-3" />
              </svg>
            </button>
            <button
              onClick={handleCopy}
              className="flex items-center justify-center w-7 h-7 rounded-lg text-gray-500 hover:bg-gray-200/60 hover:text-gray-700 transition-colors"
              title="Copy"
            >
              <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                <rect x="5" y="5" width="9" height="9" rx="1.5" />
                <path d="M11 5V3.5A1.5 1.5 0 0 0 9.5 2h-6A1.5 1.5 0 0 0 2 3.5v6A1.5 1.5 0 0 0 3.5 11H5" />
              </svg>
            </button>
            <button
              onClick={handleReplace}
              className="flex items-center gap-1.5 rounded-lg bg-copilot-blue px-3 py-1.5 text-xs font-medium text-white hover:bg-copilot-blue-hover transition-colors ml-1"
            >
              <svg className="w-3 h-3" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M10 2L4.5 7.5 2 5" />
              </svg>
              Replace
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default Popup;
