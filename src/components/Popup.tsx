import { useState, useCallback, useEffect, useMemo, useRef, type FC } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { marked } from "marked";
import iconImg from "../assets/icon-48.png";
import { SelectionInfo, ProcessResponse } from "../hooks/useSelection";

type PopupState = "icon" | "spinning" | "expanded" | "error";

interface CopilotModel {
  id: string;
  name: string;
}

interface PopupProps {
  selection: SelectionInfo | null;
}

const Popup: FC<PopupProps> = ({ selection }) => {
  const [state, setState] = useState<PopupState>("icon");
  const [result, setResult] = useState<ProcessResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showOriginal, setShowOriginal] = useState(false);
  const [showRaw, setShowRaw] = useState(false);
  const [currentModel, setCurrentModel] = useState<string>("");
  const [beastMode, setBeastMode] = useState<boolean>(false);
  const [replaceMode, setReplaceMode] = useState<"rendered" | "markdown">("rendered");
  const [showReplaceMenu, setShowReplaceMenu] = useState(false);

  // Refresh settings (model name + beast mode) from backend
  const refreshSettings = useCallback(async () => {
    try {
      const s = await invoke<{ model: string; beast_mode: boolean; replace_mode: string }>("get_settings");
      setBeastMode(s.beast_mode || false);
      setReplaceMode((s.replace_mode === "markdown" ? "markdown" : "rendered") as "rendered" | "markdown");
      if (!s.model) { setCurrentModel(""); return; }
      try {
        const models = await invoke<CopilotModel[]>("list_models");
        const match = models.find((m) => m.id === s.model);
        setCurrentModel(match ? match.name : s.model);
      } catch {
        setCurrentModel(s.model);
      }
    } catch { /* ignore */ }
  }, []);

  // Listen for backend events
  useEffect(() => {
    const unResult = listen<ProcessResponse>("show-preview-result", (event) => {
      setResult(event.payload);
      setError(null);
      setState("expanded");
      setRefreshing(false);
      refreshingRef.current = false;
    });

    const unLoading = listen("show-preview-loading", () => {
      // When refreshing, stay in expanded state — button shows its own spinner
      if (refreshingRef.current) return;
      setState("spinning");
      setError(null);
      setResult(null);
    });

    const unError = listen<string>("show-preview-error", (event) => {
      setError(event.payload);
      setState("error");
      setRefreshing(false);
      refreshingRef.current = false;
    });

    // Reset to icon state when popup is hidden and re-shown
    const unSelection = listen("selection-detected", () => {
      setState("icon");
      setResult(null);
      setError(null);
    });

    // Handle backend-initiated cancellation
    const unCancelled = listen("request-cancelled", () => {
      setState("icon");
      setRefreshing(false);
      refreshingRef.current = false;
    });

    return () => {
      unResult.then((f) => f());
      unLoading.then((f) => f());
      unError.then((f) => f());
      unSelection.then((f) => f());
      unCancelled.then((f) => f());
    };
  }, []);

  // Load settings on mount
  useEffect(() => {
    refreshSettings();
  }, [refreshSettings]);

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

  // Dismiss on Escape; close replace menu on outside click
  const replaceMenuRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (showReplaceMenu) { setShowReplaceMenu(false); return; }
        handleDismiss();
      }
    };
    const handleClick = (e: MouseEvent) => {
      if (showReplaceMenu && replaceMenuRef.current && !replaceMenuRef.current.contains(e.target as Node)) {
        setShowReplaceMenu(false);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("mousedown", handleClick);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("mousedown", handleClick);
    };
  }, [showReplaceMenu]);

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

  // Ref for expanded content container to measure actual height
  const contentRef = useRef<HTMLDivElement>(null);

  // Resize popup window to fit rendered content (only on initial expand, not refresh)
  const hasResized = useRef(false);
  useEffect(() => {
    if (state !== "expanded" || !contentRef.current) return;
    // Skip resize if this is a refresh result (keep current size)
    if (hasResized.current) return;
    // Wait a tick for DOM to settle after render
    const timer = setTimeout(() => {
      if (contentRef.current) {
        // Temporarily remove maxHeight to measure natural content height
        const card = contentRef.current;
        const oldMaxH = card.style.maxHeight;
        card.style.maxHeight = "none";
        const totalHeight = card.scrollHeight;
        card.style.maxHeight = oldMaxH;
        // Clamp and tell backend to resize
        invoke("resize_popup_content", { height: Math.min(Math.max(totalHeight, 80), 400) }).catch(() => {});
        hasResized.current = true;
      }
    }, 50);
    return () => clearTimeout(timer);
  }, [state, translatedHtml, reorganizedHtml]); // Only resize on content change, NOT on toggle/showRaw

  const handleIconClick = useCallback(async () => {
    // Check auth status fresh on every click — picks up login done in Settings
    try {
      const auth = await invoke<{ logged_in: boolean }>("get_auth_status");
      if (!auth.logged_in) {
        invoke("log_action", { action: "Icon clicked — not logged in, opening Settings" }).catch(() => {});
        await invoke("open_settings");
        return;
      }
    } catch {
      setError("Please login via tray → Settings");
      setState("error");
      return;
    }

    if (!selection) return;

    invoke("log_action", { action: `Icon clicked — starting translation (${selection.text.length} chars)` }).catch(() => {});
    // Refresh settings display (model name + beast mode) before sending request
    refreshSettings();

    setState("spinning");
    try {
      await invoke("process_and_show_preview", {
        request: { text: selection.text, action: "TranslateAndPolish" },
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setState("error");
    }
  }, [selection, refreshSettings]);

  const [refreshing, setRefreshing] = useState(false);
  const refreshingRef = useRef(false);

  const handleRefresh = useCallback(async () => {
    if (!selection || refreshing) return;
    invoke("log_action", { action: "Refresh clicked" }).catch(() => {});
    setRefreshing(true);
    refreshingRef.current = true;
    setError(null);
    try {
      await invoke("process_and_show_preview", {
        request: { text: selection.text, action: "TranslateAndPolish", is_refresh: true },
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setState("error");
      setRefreshing(false);
      refreshingRef.current = false;
    }
  }, [selection, refreshing]);

  const handleReplace = useCallback(async () => {
    if (!outputText) return;
    try {
      const html = replaceMode === "rendered" && translatedHtml ? translatedHtml : null;
      await invoke("log_action", { action: `Replace clicked — mode=${replaceMode}, text_len=${outputText.length}` }).catch(() => {});
      await invoke("replace_text", { text: outputText, html });
      await invoke("dismiss_popup");
      resetState();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [outputText, translatedHtml, replaceMode]);

  const switchReplaceMode = useCallback(async (mode: "rendered" | "markdown") => {
    setReplaceMode(mode);
    setShowReplaceMenu(false);
    await invoke("log_action", { action: `Replace mode changed to: ${mode}` }).catch(() => {});
    // Persist the setting
    try {
      const s = await invoke<Record<string, unknown>>("get_settings");
      await invoke("update_settings", { settings: { ...s, replace_mode: mode } });
    } catch { /* non-critical */ }
  }, []);

  const handleCopy = useCallback(async () => {
    if (!outputText) return;
    try {
      if (replaceMode === "rendered" && translatedHtml) {
        await invoke("log_action", { action: `Copy clicked — mode=rendered, text_len=${outputText.length}` }).catch(() => {});
        await invoke("copy_html_to_clipboard", { html: translatedHtml, text: outputText });
      } else {
        await invoke("log_action", { action: `Copy clicked — mode=markdown, text_len=${outputText.length}` }).catch(() => {});
        await invoke("copy_to_clipboard", { text: outputText });
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [outputText, translatedHtml, replaceMode]);

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
    setShowRaw(false);
    setRefreshing(false);
    refreshingRef.current = false;
    hasResized.current = false;
  };

  // ── Icon state (48×48) ──
  if (state === "icon") {
    return (
      <div className="w-screen h-screen flex items-center justify-center" style={{ padding: "20px", background: "transparent" }}>
        <div className="w-full h-full flex items-center justify-center"
          style={{
            background: "linear-gradient(135deg, #fff 0%, #f0f4ff 100%)",
            borderRadius: "50%",
            border: "1px solid rgba(0,120,212,0.15)",
            boxShadow: "0 4px 16px rgba(0,120,212,0.12), 0 1px 3px rgba(0,0,0,0.08)",
          }}
        >
          <button
            onPointerDown={(e) => { e.preventDefault(); handleIconClick(); }}
            className="w-full h-full flex items-center justify-center group"
            style={{ cursor: "pointer", borderRadius: "50%" }}
            title="Polish & Translate"
          >
            <img
              src={iconImg}
              alt="Translate"
              className="w-7 h-7 transition-transform group-hover:scale-110"
              draggable={false}
            />
          </button>
        </div>
      </div>
    );
  }

  // ── Spinning state (48×48 with spinner — click to cancel) ──
  if (state === "spinning") {
    return (
      <div className="w-screen h-screen flex items-center justify-center" style={{ padding: "20px", background: "transparent" }}>
        <button
          className="w-full h-full flex items-center justify-center group"
          onClick={() => {
            invoke("cancel_request").catch(() => {});
            invoke("log_action", { action: "Cancel clicked during spinning" }).catch(() => {});
            setState("icon");
          }}
          title="Click to cancel"
          style={{
            background: "linear-gradient(135deg, #fff 0%, #f0f4ff 100%)",
            borderRadius: "50%",
            border: "1px solid rgba(0,120,212,0.15)",
            boxShadow: "0 4px 16px rgba(0,120,212,0.12), 0 1px 3px rgba(0,0,0,0.08)",
            cursor: "pointer",
            transition: "all 0.15s ease",
          }}
          onMouseEnter={(e) => {
            e.currentTarget.style.borderColor = "rgba(239,68,68,0.5)";
            e.currentTarget.style.boxShadow = "0 4px 16px rgba(239,68,68,0.15), 0 1px 3px rgba(0,0,0,0.08)";
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.borderColor = "rgba(0,120,212,0.15)";
            e.currentTarget.style.boxShadow = "0 4px 16px rgba(0,120,212,0.12), 0 1px 3px rgba(0,0,0,0.08)";
          }}
        >
          <div className="h-5 w-5 animate-spin rounded-full border-2 border-copilot-blue border-t-transparent group-hover:hidden" />
          {/* Show X icon on hover */}
          <svg className="h-4 w-4 text-red-500 hidden group-hover:block" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
            <line x1="4" y1="4" x2="12" y2="12" />
            <line x1="12" y1="4" x2="4" y2="12" />
          </svg>
        </button>
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
      <div ref={contentRef} className="flex flex-col rounded-lg overflow-hidden h-full"
        style={{
          background: "#fff",
          border: "1px solid rgba(0,0,0,0.08)",
          boxShadow: "0 8px 32px rgba(0,0,0,0.12), 0 2px 8px rgba(0,0,0,0.06)",
        }}
      >
        {/* Layer 1: Translation (visible when original is collapsed) */}
        {!showOriginal && (
          <div className="flex-1 min-h-0 overflow-auto px-5 pt-5 pb-3" style={{ userSelect: "text", WebkitUserSelect: "text" }}>
            {showRaw ? (
              <pre className="text-[12px] leading-[1.6] text-gray-700 whitespace-pre-wrap break-words font-mono">{translated}</pre>
            ) : (
              <div
                className="text-[13.5px] leading-[1.7] text-gray-800 prose prose-sm max-w-none prose-p:my-1 prose-li:my-0.5 prose-ul:my-1.5 prose-ol:my-1.5 prose-headings:my-1.5 prose-strong:text-gray-900"
                dangerouslySetInnerHTML={{ __html: translatedHtml }}
              />
            )}
          </div>
        )}

        {/* Layer 2: Original expanded (fills entire content area when open) */}
        {showOriginal && (
          <div className="flex-1 min-h-0 overflow-auto px-5 pt-5 pb-3" style={{ userSelect: "text", WebkitUserSelect: "text" }}>
            {showRaw ? (
              <pre className="text-[12px] leading-[1.6] text-gray-500 whitespace-pre-wrap break-words font-mono">{reorganized}</pre>
            ) : (
              <div className="text-[12px] leading-[1.55] text-gray-400 prose prose-sm max-w-none prose-p:my-0.5 prose-li:my-0.5 prose-ul:my-1 prose-ol:my-1">
                <div dangerouslySetInnerHTML={{ __html: reorganizedHtml }} />
              </div>
            )}
          </div>
        )}

        {/* Toggle bar — always visible between content and action bar */}
        {reorganizedHtml && (
          <div className="flex-shrink-0 px-5 border-t border-gray-100">
            <button
              onClick={() => {
                const next = !showOriginal;
                setShowOriginal(next);
                invoke("log_action", { action: `Original section ${next ? "expanded" : "collapsed"}` }).catch(() => {});
              }}
              className="flex items-center gap-1 text-[11px] text-gray-400 hover:text-gray-600 transition-colors py-2 w-full"
            >
              <svg
                className={`w-3 h-3 transition-transform ${showOriginal ? "rotate-90" : ""}`}
                viewBox="0 0 12 12" fill="currentColor"
              >
                <path d="M4.5 2l5 4-5 4V2z" />
              </svg>
              <span className="font-medium tracking-wide uppercase">Original (reorganized)</span>
            </button>
          </div>
        )}

        {/* Action bar — always visible at bottom */}
        <div className="flex-shrink-0 flex items-center justify-between border-t border-gray-100 px-3 py-2"
          style={{ background: "rgba(249,250,251,0.8)" }}
        >
          <div className="flex items-center gap-1">
            <button
              onClick={handleDismiss}
              className="flex items-center justify-center w-7 h-7 rounded-lg text-gray-400 hover:bg-gray-200/60 hover:text-gray-600 transition-colors"
              title="Dismiss"
            >
              <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
                <path d="M3 3l10 10M13 3L3 13" />
              </svg>
            </button>
            {currentModel && (
              <button
                onClick={() => {
                  invoke("log_action", { action: "Model name clicked — opening Settings" }).catch(() => {});
                  invoke("open_settings").catch(() => {});
                }}
                className="text-[10px] text-gray-400 font-mono truncate max-w-[120px] hover:text-copilot-blue transition-colors cursor-pointer"
                title={`${currentModel} — Click to change model`}
              >
                {currentModel}
              </button>
            )}
            {beastMode && (
              <button
                onClick={() => {
                  invoke("log_action", { action: "Beast icon clicked — opening Settings" }).catch(() => {});
                  invoke("open_settings").catch(() => {});
                }}
                className="flex items-center justify-center w-7 h-7 rounded-lg text-blue-500 bg-blue-50 hover:bg-blue-100 transition-colors cursor-pointer"
                title="Beast Mode: ON — Click to change in Settings"
              >
                <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="currentColor">
                  <path d="M1 2.5L3.5 8l-1 2.5C2.5 10.5 4 13 8 14c4-1 5.5-3.5 5.5-3.5L12.5 8 15 2.5 11.5 5 8 1 4.5 5z" />
                </svg>
              </button>
            )}
            <button
              onClick={() => {
                invoke("log_action", { action: "Settings button clicked" }).catch(() => {});
                invoke("open_settings").catch(() => {});
              }}
              className="flex items-center justify-center w-7 h-7 rounded-lg text-gray-400 hover:bg-gray-200/60 hover:text-gray-600 transition-colors"
              title="Settings"
            >
              <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="8" cy="8" r="2.5" />
                <path d="M13.5 8a5.5 5.5 0 0 0-.1-.9l1.4-1.1-1.2-2-1.7.6a5.3 5.3 0 0 0-1.6-.9L10 2H8L7.7 3.7a5.3 5.3 0 0 0-1.6.9l-1.7-.6-1.2 2 1.4 1.1a5.6 5.6 0 0 0 0 1.8l-1.4 1.1 1.2 2 1.7-.6c.5.4 1 .7 1.6.9L8 14h2l.3-1.7c.6-.2 1.1-.5 1.6-.9l1.7.6 1.2-2-1.4-1.1a5.5 5.5 0 0 0 .1-.9z" />
              </svg>
            </button>
          </div>
          <div className="flex items-center gap-1">
            {/* Markdown/Preview toggle */}
            <button
              onClick={() => {
                const next = !showRaw;
                setShowRaw(next);
                invoke("log_action", { action: `Markdown view ${next ? "ON" : "OFF"}` }).catch(() => {});
              }}
              className={`flex items-center justify-center w-7 h-7 rounded-lg transition-colors ${showRaw ? "text-blue-500 bg-blue-50 hover:bg-blue-100" : "text-gray-400 hover:bg-gray-200/60 hover:text-gray-600"}`}
              title={showRaw ? "Show preview" : "Show markdown"}
            >
              <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="currentColor">
                <path d="M2.5 3A1.5 1.5 0 0 0 1 4.5v7A1.5 1.5 0 0 0 2.5 13h11a1.5 1.5 0 0 0 1.5-1.5v-7A1.5 1.5 0 0 0 13.5 3h-11zM3 9V7l1.5 2L6 7v2h1V5H6L4.5 7.5 3 5H2v4h1zm7-1h1.5L9.5 11V8H8V5h1v3z" />
              </svg>
            </button>
            <button
              onClick={refreshing ? () => {
                invoke("cancel_request").catch(() => {});
                invoke("log_action", { action: "Cancel clicked during refresh" }).catch(() => {});
                setRefreshing(false);
                refreshingRef.current = false;
              } : handleRefresh}
              className={`flex items-center justify-center w-7 h-7 rounded-lg transition-colors ${refreshing ? "text-red-400 hover:bg-red-50 hover:text-red-500 cursor-pointer" : "text-gray-500 hover:bg-gray-200/60 hover:text-gray-700"}`}
              title={refreshing ? "Cancel" : "Regenerate"}
            >
              {refreshing ? (
                <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                  <rect x="3" y="3" width="10" height="10" rx="1" fill="currentColor" stroke="none" />
                </svg>
              ) : (
                <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M2.5 8a5.5 5.5 0 0 1 9.3-4" />
                  <path d="M13.5 8a5.5 5.5 0 0 1-9.3 4" />
                  <path d="M11.5 1.5v3h3" />
                  <path d="M4.5 14.5v-3h-3" />
                </svg>
              )}
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
            <div className="relative ml-1 flex" ref={replaceMenuRef}>
              <button
                onClick={handleReplace}
                className="flex items-center gap-1.5 rounded-l-lg bg-copilot-blue px-3 py-1.5 text-xs font-medium text-white hover:bg-copilot-blue-hover transition-colors"
                title={replaceMode === "rendered" ? "Replace with rendered text" : "Replace with markdown"}
              >
                {replaceMode === "rendered" ? (
                  /* Rich text icon — lines with formatting (bold first line) */
                  <svg className="w-3.5 h-3.5 shrink-0" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
                    <path d="M2 3h12" strokeWidth="2.2" />
                    <path d="M2 6.5h9" />
                    <path d="M2 10h11" />
                    <path d="M2 13.5h7" />
                  </svg>
                ) : (
                  /* Markdown icon — "MD" in a rounded box */
                  <svg className="w-3.5 h-3.5 shrink-0" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round">
                    <rect x="0.5" y="3" width="15" height="10" rx="2" />
                    <path d="M3.5 10V6L5.5 8.5 7.5 6v4" />
                    <path d="M10.5 10V6l2.5 4V6" />
                  </svg>
                )}
                Replace
              </button>
              <button
                onClick={(e) => { e.stopPropagation(); setShowReplaceMenu(!showReplaceMenu); }}
                className="flex items-center rounded-r-lg bg-copilot-blue px-1.5 py-1.5 text-white hover:bg-copilot-blue-hover transition-colors border-l border-white/20"
                title="Replace options"
              >
                <svg className="w-2.5 h-2.5" viewBox="0 0 10 10" fill="currentColor">
                  <path d="M2 3.5L5 6.5L8 3.5" />
                </svg>
              </button>
              {showReplaceMenu && (
                <div className="absolute bottom-full right-0 mb-1 bg-white rounded-lg shadow-lg border border-gray-200 py-1 min-w-[180px] z-50">
                  <button
                    onClick={(e) => { e.stopPropagation(); switchReplaceMode("rendered"); }}
                    className={`w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 flex items-center gap-2 ${replaceMode === "rendered" ? "text-copilot-blue font-medium" : "text-gray-700"}`}
                  >
                    {replaceMode === "rendered" && <span>✓</span>}
                    <span className={replaceMode !== "rendered" ? "ml-5" : ""}>Rendered text</span>
                  </button>
                  <button
                    onClick={(e) => { e.stopPropagation(); switchReplaceMode("markdown"); }}
                    className={`w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 flex items-center gap-2 ${replaceMode === "markdown" ? "text-copilot-blue font-medium" : "text-gray-700"}`}
                  >
                    {replaceMode === "markdown" && <span>✓</span>}
                    <span className={replaceMode !== "markdown" ? "ml-5" : ""}>Markdown</span>
                  </button>
                </div>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};

export default Popup;
