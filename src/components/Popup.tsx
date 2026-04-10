import { useState, useCallback, useEffect, useMemo, useRef, type FC } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { marked } from "marked";
import "github-markdown-css/github-markdown-light.css";
import iconImg from "../assets/icon-48.png";
import { SelectionInfo, ProcessResponse } from "../hooks/useSelection";

type PopupState = "icon" | "spinning" | "streaming" | "expanded" | "error";

interface CopilotModel {
  id: string;
  name: string;
}

interface PopupProps {
  selection: SelectionInfo | null;
}

/** Find the index of the first unescaped double-quote in a string. Returns -1 if not found. */
function findUnescapedQuote(s: string): number {
  for (let i = 0; i < s.length; i++) {
    if (s[i] === '\\') { i++; continue; } // skip escaped char
    if (s[i] === '"') return i;
  }
  return -1;
}

const Popup: FC<PopupProps> = ({ selection }) => {
  const [state, setState] = useState<PopupState>("icon");
  const stateRef = useRef<PopupState>("icon");
  const [streamingText, setStreamingText] = useState<string | null>(null);
  const [result, setResult] = useState<ProcessResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showOriginal, setShowOriginal] = useState(false);
  const [showRaw, setShowRaw] = useState(false);
  const [currentModel, setCurrentModel] = useState<string>("");
  const [beastMode, setBeastMode] = useState<boolean>(false);
  const [replaceMode, setReplaceMode] = useState<"rendered" | "markdown">("rendered");
  const [showReplaceMenu, setShowReplaceMenu] = useState(false);

  // Read Mode state
  const [isReadMode, setIsReadMode] = useState(false);

  // Keep stateRef in sync for use in event listener closures
  const setPopupState = useCallback((s: PopupState) => {
    stateRef.current = s;
    setState(s);
  }, []);
  const [readModeSettings, setReadModeSettings] = useState<{
    native_language: string;
    target_language: string;
    read_mode_sub: string;
  }>({ native_language: "Chinese (Simplified)", target_language: "English", read_mode_sub: "translate_summarize" });

  // Apply theme to this window's <html> based on settings
  const applyTheme = useCallback(async (themeValue?: string) => {
    let resolved: "light" | "dark" = "light";
    const t = themeValue || "system";
    if (t === "dark") {
      resolved = "dark";
    } else if (t === "light") {
      resolved = "light";
    } else {
      // "system" — detect OS preference
      try {
        const sys = await invoke<string>("get_system_theme");
        resolved = sys === "dark" ? "dark" : "light";
      } catch {
        resolved = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
      }
    }
    if (resolved === "dark") {
      document.documentElement.classList.add("dark");
    } else {
      document.documentElement.classList.remove("dark");
    }
  }, []);

  // Track dark mode from <html> class
  const [isDark, setIsDark] = useState(() => document.documentElement.classList.contains("dark"));
  useEffect(() => {
    const observer = new MutationObserver(() => {
      setIsDark(document.documentElement.classList.contains("dark"));
    });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });
    return () => observer.disconnect();
  }, []);

  // Refresh settings (model name + beast mode + theme + read mode) from backend
  const refreshSettings = useCallback(async () => {
    try {
      const s = await invoke<{
        model: string; beast_mode: boolean; replace_mode: string; theme?: string;
        native_language?: string; target_language?: string; read_mode_enabled?: boolean; read_mode_sub?: string;
      }>("get_settings");
      setBeastMode(s.beast_mode || false);
      applyTheme(s.theme);
      setReplaceMode((s.replace_mode === "markdown" ? "markdown" : "rendered") as "rendered" | "markdown");
      setReadModeSettings({
        native_language: s.native_language || "Chinese (Simplified)",
        target_language: s.target_language || "English",
        read_mode_sub: s.read_mode_sub || "translate_summarize",
      });
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
      setStreamingText(null); // Clear streaming state on final result
      setResult(event.payload);
      setError(null);
      setPopupState("expanded");
      setRefreshing(false);
      refreshingRef.current = false;
    });

    const unLoading = listen("show-preview-loading", () => {
      // When refreshing, stay in expanded/streaming state — button shows its own spinner
      if (refreshingRef.current) return;
      setPopupState("spinning");
      setError(null);
      setResult(null);
      setStreamingText(null);
    });

    // Listen for incremental streaming chunks
    const unChunk = listen<string>("show-preview-chunk", (event) => {
      setStreamingText(event.payload);
      // Transition from spinning to streaming on first chunk
      if (stateRef.current === "spinning") {
        setPopupState("streaming");
      }
    });

    const unError = listen<string>("show-preview-error", (event) => {
      setStreamingText(null);
      setError(event.payload);
      setPopupState("error");
      setRefreshing(false);
      refreshingRef.current = false;
    });

    const unSelection = listen("selection-detected", () => {
      setPopupState("icon");
      setResult(null);
      setError(null);
      setStreamingText(null);
      refreshSettings(); // pick up model/settings changes
    });

    // Handle backend-initiated cancellation
    const unCancelled = listen("request-cancelled", () => {
      setPopupState("icon");
      setStreamingText(null);
      setRefreshing(false);
      refreshingRef.current = false;
    });

    return () => {
      unResult.then((f) => f());
      unLoading.then((f) => f());
      unChunk.then((f) => f());
      unError.then((f) => f());
      unSelection.then((f) => f());
      unCancelled.then((f) => f());
    };
  }, []);

  // Load settings on mount
  useEffect(() => {
    refreshSettings();
  }, [refreshSettings]);

  // Parse result — handles both Write Mode and Read Mode JSON formats
  // Write Mode: {"reorganized": "...", "translated": "..."}
  // Read Mode (smart): {"mode": "word|simple|complex|long", ...}
  const { reorganized, translated, readModeType, readTargetLang, readTranslation, readSummary, readExplanation, readExamples, readVocabulary } = useMemo(() => {
    const empty = { reorganized: "", translated: "", readModeType: "" as string, readTargetLang: "", readTranslation: "", readSummary: "", readExplanation: "", readExamples: [] as string[], readVocabulary: [] as { term: string; meaning: string; usage: string }[] };
    if (!result?.result) return empty;
    const text = result.result.trim();
    try {
      // Strip markdown code fences if LLM wraps in ```json ... ```
      const cleaned = text.replace(/^```(?:json)?\s*\n?/i, "").replace(/\n?```\s*$/, "");
      const parsed = JSON.parse(cleaned);

      // Read Mode smart format (new unified format)
      if (parsed.mode) {
        const mode = String(parsed.mode);
        return {
          reorganized: "",
          translated: "",
          readModeType: mode,
          readTargetLang: String(parsed.target || ""),
          readTranslation: String(parsed.translation || ""),
          readSummary: String(parsed.summary || ""),
          readExplanation: String(parsed.explanation || ""),
          readExamples: Array.isArray(parsed.examples) ? parsed.examples.map(String) : [],
          readVocabulary: Array.isArray(parsed.vocabulary) ? parsed.vocabulary : [],
        };
      }

      // Legacy Read Mode format (summary + translation)
      if (parsed.summary && parsed.translation) {
        return {
          ...empty,
          readModeType: "long",
          readSummary: String(parsed.summary),
          readTranslation: String(parsed.translation),
        };
      }

      // Write Mode format
      if (parsed.reorganized && parsed.translated) {
        return {
          ...empty,
          reorganized: String(parsed.reorganized),
          translated: String(parsed.translated),
        };
      }
    } catch {
      // JSON parse failed — fallback: try "---" divider (Write Mode only)
      if (!isReadMode) {
        const dividerMatch = text.match(/\n---\n/);
        if (dividerMatch && dividerMatch.index !== undefined) {
          return {
            ...empty,
            reorganized: text.slice(0, dividerMatch.index).trim(),
            translated: text.slice(dividerMatch.index + dividerMatch[0].length).trim(),
          };
        }
      }
    }
    // No structure found — treat entire text as the main result
    if (isReadMode) {
      return { ...empty, readModeType: "simple", readTranslation: text };
    }
    return { ...empty, translated: text };
  }, [result?.result, isReadMode]);

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

  // Read Mode markdown
  const readSummaryHtml = useMemo(() => {
    if (!readSummary) return "";
    marked.setOptions({ breaks: true, gfm: true });
    return marked.parse(readSummary) as string;
  }, [readSummary]);

  const readTranslationHtml = useMemo(() => {
    if (!readTranslation) return "";
    marked.setOptions({ breaks: true, gfm: true });
    return marked.parse(readTranslation) as string;
  }, [readTranslation]);

  const readExplanationHtml = useMemo(() => {
    if (!readExplanation) return "";
    marked.setOptions({ breaks: true, gfm: true });
    return marked.parse(readExplanation) as string;
  }, [readExplanation]);

  // Extract displayable text from streaming content.
  // The LLM often returns JSON (Write Mode: {"reorganized":"...","translated":"..."},
  // Read Mode: {"mode":"...","translation":"..."}). During streaming the JSON is incomplete,
  // so we extract the last string value being written to show meaningful content.
  const streamingDisplayText = useMemo(() => {
    if (!streamingText) return "";
    const text = streamingText.trim();

    // If it doesn't look like JSON, render as-is (plain text modes like Translate/Polish)
    if (!text.startsWith("{")) return text;

    // Try to extract content from partial JSON.
    // Strategy: find the last key-value pair being written and extract its string value.
    // For Write Mode: we want "translated" (or "reorganized" if "translated" hasn't started)
    // For Read Mode: we want "translation" (or "summary", "explanation")

    // Priority order of keys to extract (last wins — we want the one currently streaming)
    const keys = ["reorganized", "translated", "summary", "explanation", "translation"];
    let bestContent = "";
    for (const key of keys) {
      const marker = `"${key}"`;
      const idx = text.indexOf(marker);
      if (idx === -1) continue;
      // Find the start of the string value after the key
      const afterKey = text.substring(idx + marker.length);
      // Skip :\s*"
      const valMatch = afterKey.match(/^\s*:\s*"/);
      if (!valMatch) continue;
      const valueStart = idx + marker.length + valMatch[0].length;
      // Extract everything from valueStart to end, handling escaped quotes
      let raw = text.substring(valueStart);
      // If the value is properly closed, trim the closing quote
      // Otherwise take everything (it's still being streamed)
      const closingIdx = findUnescapedQuote(raw);
      if (closingIdx !== -1) {
        raw = raw.substring(0, closingIdx);
      }
      // Unescape JSON string escapes
      bestContent = raw.replace(/\\n/g, "\n").replace(/\\"/g, '"').replace(/\\\\/g, "\\");
    }

    return bestContent || text; // Fallback to raw text if no key found
  }, [streamingText]);

  // Streaming markdown — render extracted content as markdown
  const streamingHtml = useMemo(() => {
    if (!streamingDisplayText) return "";
    marked.setOptions({ breaks: true, gfm: true });
    return marked.parse(streamingDisplayText) as string;
  }, [streamingDisplayText]);

  // The text to use for Replace/Copy
  // Write Mode: translated text; Read Mode: all relevant content
  const outputText = isReadMode
    ? (readSummary || readTranslation || readExplanation || result?.result || "")
    : (translated || result?.result || "");
  const outputHtml = isReadMode
    ? (readSummaryHtml || readTranslationHtml || readExplanationHtml)
    : translatedHtml;

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

  // Auto-dismiss on blur (when expanded or streaming)
  useEffect(() => {
    if (state !== "expanded" && state !== "streaming" && state !== "error") return;
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

  // Resize popup window to fit rendered content
  // During streaming: throttled to max once per 200ms
  // On initial expand: once after 50ms
  const hasResized = useRef(false);
  useEffect(() => {
    if ((state !== "expanded" && state !== "streaming") || !contentRef.current) return;
    // For expanded state: skip resize if this is a refresh result (keep current size)
    if (state === "expanded" && hasResized.current) return;
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
        if (state === "expanded") {
          hasResized.current = true;
        }
      }
    }, state === "streaming" ? 200 : 50);
    return () => clearTimeout(timer);
  }, [state, streamingText, translatedHtml, reorganizedHtml, readSummaryHtml, readTranslationHtml, readExplanationHtml, readModeType]);

  const handleIconClick = useCallback(async () => {
    // Show spinner immediately — don't wait for auth/settings checks
    setPopupState("spinning");

    // Refresh settings to pick up model/beast mode changes from Settings window
    await refreshSettings();

    // Check auth status fresh on every click — picks up login done in Settings
    try {
      const auth = await invoke<{ logged_in: boolean }>("get_auth_status");
      if (!auth.logged_in) {
        invoke("log_action", { action: "Icon clicked — not logged in, opening Settings" }).catch(() => {});
        setPopupState("icon"); // revert spinner
        await invoke("open_settings");
        return;
      }
    } catch {
      setError("Please login via tray → Settings");
      setPopupState("error");
      return;
    }

    if (!selection) {
      setPopupState("icon"); // revert spinner
      return;
    }

    // Detect Read Mode vs Write Mode based on selection source
    const readMode = selection.is_input_element === false;
    setIsReadMode(readMode);

    invoke("log_action", { action: `Icon clicked — ${readMode ? "Read" : "Write"} Mode (${selection.text.length} chars)` }).catch(() => {});

    try {
      if (readMode) {
        // Read Mode: determine translation direction
        // If selected text != native language → translate to native; if == native → translate to target
        // We send both options and let the frontend pass the resolved language
        const summarize = readModeSettings.read_mode_sub === "translate_summarize";
        await invoke("process_and_show_preview", {
          request: {
            text: selection.text,
            action: "ReadModeTranslate",
            read_target_language: readModeSettings.native_language,
            read_summarize: summarize,
          },
        });
      } else {
        // Write Mode: existing behavior
        await invoke("process_and_show_preview", {
          request: { text: selection.text, action: "TranslateAndPolish" },
        });
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      // Ignore cancellation — already handled by cancel button
      if (msg.includes("cancelled")) return;
      setError(msg);
      setPopupState("error");
    }
  }, [selection, refreshSettings, readModeSettings]);

  const [refreshing, setRefreshing] = useState(false);
  const [copyToast, setCopyToast] = useState(false);
  const refreshingRef = useRef(false);

  const handleRefresh = useCallback(async () => {
    if (!selection || refreshing) return;
    invoke("log_action", { action: "Refresh clicked" }).catch(() => {});
    setRefreshing(true);
    refreshingRef.current = true;
    setError(null);
    try {
      if (isReadMode) {
        const summarize = readModeSettings.read_mode_sub === "translate_summarize";
        await invoke("process_and_show_preview", {
          request: {
            text: selection.text,
            action: "ReadModeTranslate",
            is_refresh: true,
            read_target_language: readModeSettings.native_language,
            read_summarize: summarize,
          },
        });
      } else {
        await invoke("process_and_show_preview", {
          request: { text: selection.text, action: "TranslateAndPolish", is_refresh: true },
        });
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      // Ignore cancellation — already handled by cancel button
      if (msg.includes("cancelled")) {
        setRefreshing(false);
        refreshingRef.current = false;
        return;
      }
      setError(msg);
      setPopupState("error");
      setRefreshing(false);
      refreshingRef.current = false;
    }
  }, [selection, refreshing, isReadMode, readModeSettings]);

  const handleReplace = useCallback(async () => {
    if (!outputText) return;
    try {
      const html = replaceMode === "rendered" && outputHtml ? outputHtml : null;
      await invoke("log_action", { action: `Replace clicked — mode=${replaceMode}, text_len=${outputText.length}` }).catch(() => {});
      await invoke("replace_text", { text: outputText, html });
      await invoke("dismiss_popup");
      resetState();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [outputText, outputHtml, replaceMode]);

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
      if (replaceMode === "rendered" && outputHtml) {
        await invoke("log_action", { action: `Copy clicked — mode=rendered, text_len=${outputText.length}` }).catch(() => {});
        await invoke("copy_html_to_clipboard", { html: outputHtml, text: outputText });
      } else {
        await invoke("log_action", { action: `Copy clicked — mode=markdown, text_len=${outputText.length}` }).catch(() => {});
        await invoke("copy_to_clipboard", { text: outputText });
      }
      setCopyToast(true);
      setTimeout(() => setCopyToast(false), 1500);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [outputText, outputHtml, replaceMode]);

  const handleDismiss = useCallback(async () => {
    // Cancel any in-flight request when dismissing during streaming/spinning
    if (stateRef.current === "streaming" || stateRef.current === "spinning") {
      invoke("cancel_request").catch(() => {});
    }
    try {
      await invoke("dismiss_popup");
    } catch {}
    resetState();
  }, []);

  const resetState = () => {
    setPopupState("icon");
    setResult(null);
    setError(null);
    setStreamingText(null);
    setShowOriginal(false);
    setShowRaw(false);
    setRefreshing(false);
    refreshingRef.current = false;
    hasResized.current = false;
    setIsReadMode(false);
  };

  // ── Icon state (48×48) ──
  if (state === "icon") {
    return (
      <div className="w-screen h-screen flex items-center justify-center" style={{ padding: "20px", background: "transparent" }}>
        <div className="w-full h-full flex items-center justify-center"
          style={{
            background: isDark
              ? "linear-gradient(135deg, #1e293b 0%, #0f172a 100%)"
              : "linear-gradient(135deg, #fff 0%, #f0f4ff 100%)",
            borderRadius: "50%",
            border: isDark
              ? "1px solid rgba(96,165,250,0.25)"
              : "1px solid rgba(0,120,212,0.15)",
            boxShadow: isDark
              ? "0 4px 16px rgba(0,0,0,0.4), 0 1px 3px rgba(0,0,0,0.3)"
              : "0 4px 16px rgba(0,120,212,0.12), 0 1px 3px rgba(0,0,0,0.08)",
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
            setPopupState("icon");
          }}
          title="Click to cancel"
          style={{
            background: isDark
              ? "linear-gradient(135deg, #1e293b 0%, #0f172a 100%)"
              : "linear-gradient(135deg, #fff 0%, #f0f4ff 100%)",
            borderRadius: "50%",
            border: isDark
              ? "1px solid rgba(96,165,250,0.25)"
              : "1px solid rgba(0,120,212,0.15)",
            boxShadow: isDark
              ? "0 4px 16px rgba(0,0,0,0.4), 0 1px 3px rgba(0,0,0,0.3)"
              : "0 4px 16px rgba(0,120,212,0.12), 0 1px 3px rgba(0,0,0,0.08)",
            cursor: "pointer",
            transition: "all 0.15s ease",
          }}
          onMouseEnter={(e) => {
            e.currentTarget.style.borderColor = "rgba(239,68,68,0.5)";
            e.currentTarget.style.boxShadow = "0 4px 16px rgba(239,68,68,0.15), 0 1px 3px rgba(0,0,0,0.08)";
          }}
          onMouseLeave={(e) => {
            e.currentTarget.style.borderColor = isDark ? "rgba(96,165,250,0.25)" : "rgba(0,120,212,0.15)";
            e.currentTarget.style.boxShadow = isDark
              ? "0 4px 16px rgba(0,0,0,0.4), 0 1px 3px rgba(0,0,0,0.3)"
              : "0 4px 16px rgba(0,120,212,0.12), 0 1px 3px rgba(0,0,0,0.08)";
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

  // ── Streaming state (expanded card with partial content + blinking cursor) ──
  if (state === "streaming") {
    return (
      <div className="w-screen h-screen" style={{ padding: "20px", background: "transparent" }}>
        <div ref={contentRef} className="flex flex-col rounded-lg overflow-hidden h-full"
          style={{
            background: isDark ? "#1e293b" : "#fff",
            border: isDark ? "1px solid rgba(255,255,255,0.1)" : "1px solid rgba(0,0,0,0.08)",
            boxShadow: isDark
              ? "0 8px 32px rgba(0,0,0,0.4), 0 2px 8px rgba(0,0,0,0.3)"
              : "0 8px 32px rgba(0,0,0,0.12), 0 2px 8px rgba(0,0,0,0.06)",
          }}
        >
          <div className="flex-1 min-h-0 overflow-auto px-5 pt-5 pb-3" style={{ userSelect: "text", WebkitUserSelect: "text" }}>
            <div
              className="markdown-body text-[13.5px] leading-[1.7] streaming-content"
              style={{ background: "transparent" }}
              dangerouslySetInnerHTML={{ __html: streamingHtml }}
            />
            {/* Blinking cursor indicator */}
            <span className="inline-block w-[2px] h-[1em] bg-copilot-blue align-text-bottom animate-pulse ml-0.5" />
          </div>
          {/* Minimal action bar during streaming — just dismiss and cancel */}
          <div className="flex-shrink-0 flex items-center justify-between border-t border-gray-100 dark:border-gray-700 px-3 py-2"
            style={{ background: isDark ? "rgba(15,23,42,0.8)" : "rgba(249,250,251,0.8)" }}
          >
            <div className="flex items-center gap-1">
              <button
                onClick={handleDismiss}
                className="flex items-center justify-center w-7 h-7 rounded-lg text-gray-400 dark:text-gray-500 hover:bg-gray-200/60 dark:hover:bg-gray-700/60 hover:text-gray-600 dark:hover:text-gray-300 transition-colors"
                title="Dismiss"
              >
                <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
                  <path d="M3 3l10 10M13 3L3 13" />
                </svg>
              </button>
              {currentModel && (
                <span className="text-[10px] text-gray-400 dark:text-gray-500 font-mono truncate max-w-[120px]">
                  {currentModel}
                </span>
              )}
            </div>
            <div className="flex items-center gap-1">
              {/* Cancel/stop streaming button */}
              <button
                onClick={() => {
                  invoke("cancel_request").catch(() => {});
                  invoke("log_action", { action: "Cancel clicked during streaming" }).catch(() => {});
                }}
                className="group flex items-center justify-center w-7 h-7 rounded-lg bg-blue-50 dark:bg-blue-900/30 text-copilot-blue hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/30 transition-colors cursor-pointer"
                title="Stop generating"
              >
                <div className="h-3.5 w-3.5 animate-spin rounded-full border-[1.5px] border-copilot-blue border-t-transparent group-hover:hidden" />
                <svg className="w-3.5 h-3.5 hidden group-hover:block" viewBox="0 0 16 16" fill="currentColor">
                  <rect x="3" y="3" width="10" height="10" rx="1" />
                </svg>
              </button>
            </div>
          </div>
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
            background: isDark ? "#1e293b" : "#fff",
            border: isDark ? "1px solid rgba(255,255,255,0.1)" : "1px solid rgba(0,0,0,0.08)",
            boxShadow: isDark
              ? "0 8px 32px rgba(0,0,0,0.4), 0 2px 8px rgba(0,0,0,0.3)"
              : "0 8px 32px rgba(0,0,0,0.12), 0 2px 8px rgba(0,0,0,0.06)",
          }}
        >
          <div className="px-5 py-4 flex items-start gap-2.5">
            <div className="flex-shrink-0 w-5 h-5 rounded-full bg-red-50 dark:bg-red-900/30 flex items-center justify-center mt-0.5">
              <svg className="w-3 h-3 text-red-500" viewBox="0 0 12 12" fill="currentColor">
                <path d="M6 0a6 6 0 100 12A6 6 0 006 0zm.75 9h-1.5V7.5h1.5V9zm0-3h-1.5V3h1.5v3z"/>
              </svg>
            </div>
            <p className="text-[13px] leading-[1.5] text-red-600 dark:text-red-400">{error}</p>
          </div>
          <div className="flex justify-end border-t border-gray-100 dark:border-gray-700 px-3 py-2"
            style={{ background: isDark ? "rgba(15,23,42,0.8)" : "rgba(249,250,251,0.8)" }}
          >
            <button
              onClick={handleDismiss}
              className="rounded-lg px-3 py-1.5 text-xs font-medium text-gray-500 dark:text-gray-400 hover:bg-gray-200/60 dark:hover:bg-gray-700/60 transition-colors"
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
          background: isDark ? "#1e293b" : "#fff",
          border: isDark ? "1px solid rgba(255,255,255,0.1)" : "1px solid rgba(0,0,0,0.08)",
          boxShadow: isDark
            ? "0 8px 32px rgba(0,0,0,0.4), 0 2px 8px rgba(0,0,0,0.3)"
            : "0 8px 32px rgba(0,0,0,0.12), 0 2px 8px rgba(0,0,0,0.06)",
        }}
      >
        {isReadMode ? (
          /* ── Read Mode Content — 4 smart modes ── */
          <>
            {!showOriginal && (
              <div className="flex-1 min-h-0 overflow-auto px-5 pt-5 pb-3" style={{ userSelect: "text", WebkitUserSelect: "text" }}>

                {/* Word mode: translation + explanation + examples */}
                {readModeType === "word" && (
                  <div className="space-y-3">
                    {/* Translation */}
                    <div className="text-[15px] font-semibold text-gray-800 dark:text-gray-200">
                      {readTranslation}
                    </div>
                    {/* Explanation */}
                    {readExplanation && (
                      showRaw ? (
                        <pre className="text-[12px] leading-[1.6] text-gray-600 dark:text-gray-400 whitespace-pre-wrap break-words font-mono">{readExplanation}</pre>
                      ) : (
                        <div
                          className="markdown-body text-[13px] leading-[1.65] text-gray-600 dark:text-gray-400"
                          style={{ background: "transparent" }}
                          dangerouslySetInnerHTML={{ __html: readExplanationHtml }}
                        />
                      )
                    )}
                    {/* Examples */}
                    {readExamples.length > 0 && (
                      <div className="space-y-1.5 border-t border-gray-100 dark:border-gray-700 pt-2">
                        <div className="text-[11px] font-medium text-gray-400 dark:text-gray-500 uppercase tracking-wide">Examples</div>
                        {readExamples.map((ex: string, i: number) => (
                          <div key={i} className="text-[12.5px] leading-[1.6] text-gray-600 dark:text-gray-400 pl-3 border-l-2 border-blue-200 dark:border-blue-800">
                            {ex}
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}

                {/* Simple mode: just the translation */}
                {readModeType === "simple" && (
                  showRaw ? (
                    <pre className="text-[12px] leading-[1.6] text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words font-mono">{readTranslation}</pre>
                  ) : (
                    <div
                      className="markdown-body text-[13.5px] leading-[1.7]"
                      style={{ background: "transparent" }}
                      dangerouslySetInnerHTML={{ __html: readTranslationHtml }}
                    />
                  )
                )}

                {/* Complex mode: translation + vocabulary highlights */}
                {readModeType === "complex" && (
                  <div className="space-y-3">
                    {showRaw ? (
                      <pre className="text-[12px] leading-[1.6] text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words font-mono">{readTranslation}</pre>
                    ) : (
                      <div
                        className="markdown-body text-[13.5px] leading-[1.7]"
                        style={{ background: "transparent" }}
                        dangerouslySetInnerHTML={{ __html: readTranslationHtml }}
                      />
                    )}
                    {readVocabulary.length > 0 && (
                      <div className="space-y-2 border-t border-gray-100 dark:border-gray-700 pt-2">
                        <div className="text-[11px] font-medium text-gray-400 dark:text-gray-500 uppercase tracking-wide">📚 Vocabulary</div>
                        {readVocabulary.map((v: { term: string; meaning: string; usage: string }, i: number) => (
                          <div key={i} className="pl-3 border-l-2 border-purple-200 dark:border-purple-800">
                            <span className="text-[12.5px] font-semibold text-purple-600 dark:text-purple-400">{v.term}</span>
                            <span className="text-[12px] text-gray-600 dark:text-gray-400"> — {v.meaning}</span>
                            {v.usage && <div className="text-[11.5px] text-gray-400 dark:text-gray-500 italic mt-0.5">{v.usage}</div>}
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                )}

                {/* Long mode: summary as main content */}
                {readModeType === "long" && (
                  showRaw ? (
                    <pre className="text-[12px] leading-[1.6] text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words font-mono">{readSummary || readTranslation}</pre>
                  ) : (
                    <div
                      className="markdown-body text-[13.5px] leading-[1.7]"
                      style={{ background: "transparent" }}
                      dangerouslySetInnerHTML={{ __html: readSummaryHtml || readTranslationHtml }}
                    />
                  )
                )}

                {/* Fallback: if mode is unrecognized */}
                {!["word", "simple", "complex", "long"].includes(readModeType) && readTranslation && (
                  showRaw ? (
                    <pre className="text-[12px] leading-[1.6] text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words font-mono">{readTranslation}</pre>
                  ) : (
                    <div
                      className="markdown-body text-[13.5px] leading-[1.7]"
                      style={{ background: "transparent" }}
                      dangerouslySetInnerHTML={{ __html: readTranslationHtml }}
                    />
                  )
                )}
              </div>
            )}

            {/* Collapsible section: full translation (only in long mode) */}
            {showOriginal && readModeType === "long" && readTranslation && (
              <div className="flex-1 min-h-0 overflow-auto px-5 pt-5 pb-3" style={{ userSelect: "text", WebkitUserSelect: "text" }}>
                {showRaw ? (
                  <pre className="text-[12px] leading-[1.6] text-gray-500 dark:text-gray-400 whitespace-pre-wrap break-words font-mono">{readTranslation}</pre>
                ) : (
                  <div className="markdown-body text-[12px] leading-[1.55] text-gray-400 dark:text-gray-500" style={{ background: "transparent" }}>
                    <div dangerouslySetInnerHTML={{ __html: readTranslationHtml }} />
                  </div>
                )}
              </div>
            )}

            {/* Toggle bar — only in long mode when we have both summary and translation */}
            {readModeType === "long" && readSummary && readTranslation && (
              <div className="flex-shrink-0 px-5 border-t border-gray-100 dark:border-gray-700">
                <button
                  onClick={() => {
                    const next = !showOriginal;
                    setShowOriginal(next);
                    invoke("log_action", { action: `Full translation ${next ? "expanded" : "collapsed"}` }).catch(() => {});
                  }}
                  className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-gray-500 hover:text-gray-600 dark:hover:text-gray-300 transition-colors py-2 w-full"
                >
                  <svg
                    className={`w-3 h-3 transition-transform ${showOriginal ? "rotate-90" : ""}`}
                    viewBox="0 0 12 12" fill="currentColor"
                  >
                    <path d="M4.5 2l5 4-5 4V2z" />
                  </svg>
                  <span className="font-medium tracking-wide uppercase">{(readTargetLang || readModeSettings.native_language).split(/[\s(]/)[0]} (Full Translation)</span>
                </button>
              </div>
            )}
          </>
        ) : (
          /* ── Write Mode Content (existing) ── */
          <>
            {/* Layer 1: Translation (visible when original is collapsed) */}
            {!showOriginal && (
              <div className="flex-1 min-h-0 overflow-auto px-5 pt-5 pb-3" style={{ userSelect: "text", WebkitUserSelect: "text" }}>
                {showRaw ? (
                  <pre className="text-[12px] leading-[1.6] text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words font-mono">{translated}</pre>
                ) : (
                  <div
                    className="markdown-body text-[13.5px] leading-[1.7]"
                    style={{ background: "transparent" }}
                    dangerouslySetInnerHTML={{ __html: translatedHtml }}
                  />
                )}
              </div>
            )}

            {/* Layer 2: Original expanded (fills entire content area when open) */}
            {showOriginal && (
              <div className="flex-1 min-h-0 overflow-auto px-5 pt-5 pb-3" style={{ userSelect: "text", WebkitUserSelect: "text" }}>
                {showRaw ? (
                  <pre className="text-[12px] leading-[1.6] text-gray-500 dark:text-gray-400 whitespace-pre-wrap break-words font-mono">{reorganized}</pre>
                ) : (
                  <div className="markdown-body text-[12px] leading-[1.55] text-gray-400 dark:text-gray-500" style={{ background: "transparent" }}>
                    <div dangerouslySetInnerHTML={{ __html: reorganizedHtml }} />
                  </div>
                )}
              </div>
            )}

            {/* Toggle bar — always visible between content and action bar */}
            {reorganizedHtml && (
              <div className="flex-shrink-0 px-5 border-t border-gray-100 dark:border-gray-700">
                <button
                  onClick={() => {
                    const next = !showOriginal;
                    setShowOriginal(next);
                    invoke("log_action", { action: `Original section ${next ? "expanded" : "collapsed"}` }).catch(() => {});
                  }}
                  className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-gray-500 hover:text-gray-600 dark:hover:text-gray-300 transition-colors py-2 w-full"
                >
                  <svg
                    className={`w-3 h-3 transition-transform ${showOriginal ? "rotate-90" : ""}`}
                    viewBox="0 0 12 12" fill="currentColor"
                  >
                    <path d="M4.5 2l5 4-5 4V2z" />
                  </svg>
                  <span className="font-medium tracking-wide uppercase">{readModeSettings.native_language.split(' ')[0]} (Polished)</span>
                </button>
              </div>
            )}
          </>
        )}

        {/* Action bar — always visible at bottom */}
        <div className="flex-shrink-0 flex items-center justify-between border-t border-gray-100 dark:border-gray-700 px-3 py-2"
          style={{ background: isDark ? "rgba(15,23,42,0.8)" : "rgba(249,250,251,0.8)" }}
        >
          <div className="flex items-center gap-1">
            <button
              onClick={handleDismiss}
              className="flex items-center justify-center w-7 h-7 rounded-lg text-gray-400 dark:text-gray-500 hover:bg-gray-200/60 dark:hover:bg-gray-700/60 hover:text-gray-600 dark:hover:text-gray-300 transition-colors"
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
                className="text-[10px] text-gray-400 dark:text-gray-500 font-mono truncate max-w-[120px] hover:text-copilot-blue transition-colors cursor-pointer"
                title={`${currentModel} — Click to change model`}
              >
                {currentModel}
              </button>
            )}
            {/* Beast Mode icon — Write Mode only */}
            {!isReadMode && beastMode && (
              <button
                onClick={() => {
                  invoke("log_action", { action: "Beast icon clicked — opening Settings" }).catch(() => {});
                  invoke("open_settings").catch(() => {});
                }}
                className="flex items-center justify-center w-7 h-7 rounded-lg text-blue-500 bg-blue-50 dark:bg-blue-900/30 hover:bg-blue-100 dark:hover:bg-blue-900/50 transition-colors cursor-pointer"
                title="Beast Mode: ON — Click to change in Settings"
              >
                <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="currentColor">
                  <path d="M1 2.5L3.5 8l-1 2.5C2.5 10.5 4 13 8 14c4-1 5.5-3.5 5.5-3.5L12.5 8 15 2.5 11.5 5 8 1 4.5 5z" />
                </svg>
              </button>
            )}
            {/* Read Mode indicator with mode type */}
            {isReadMode && (
              <span className="text-[10px] text-emerald-500 dark:text-emerald-400 font-medium px-1.5 py-0.5 bg-emerald-50 dark:bg-emerald-900/30 rounded">
                {readModeType === "word" ? "📖 Word" : readModeType === "simple" ? "💬 Translate" : readModeType === "complex" ? "📚 Translate" : readModeType === "long" ? "📋 Summary" : "Read"}
              </span>
            )}
            <button
              onClick={() => {
                invoke("log_action", { action: "Settings button clicked" }).catch(() => {});
                invoke("open_settings").catch(() => {});
              }}
              className="flex items-center justify-center w-7 h-7 rounded-lg text-gray-400 dark:text-gray-500 hover:bg-gray-200/60 dark:hover:bg-gray-700/60 hover:text-gray-600 dark:hover:text-gray-300 transition-colors"
              title="Settings"
            >
              <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="8" cy="8" r="2.5" />
                <path d="M13.5 8a5.5 5.5 0 0 0-.1-.9l1.4-1.1-1.2-2-1.7.6a5.3 5.3 0 0 0-1.6-.9L10 2H8L7.7 3.7a5.3 5.3 0 0 0-1.6.9l-1.7-.6-1.2 2 1.4 1.1a5.6 5.6 0 0 0 0 1.8l-1.4 1.1 1.2 2 1.7-.6c.5.4 1 .7 1.6.9L8 14h2l.3-1.7c.6-.2 1.1-.5 1.6-.9l1.7.6 1.2-2-1.4-1.1a5.5 5.5 0 0 0 .1-.9z" />
              </svg>
            </button>
          </div>
          <div className="flex items-center gap-1">
            {/* Markdown/Preview toggle — Write Mode only */}
            {!isReadMode && (
              <button
                onClick={() => {
                  const next = !showRaw;
                  setShowRaw(next);
                  invoke("log_action", { action: `Markdown view ${next ? "ON" : "OFF"}` }).catch(() => {});
                }}
                className={`flex items-center justify-center w-7 h-7 rounded-lg transition-colors ${showRaw ? "text-blue-500 bg-blue-50 dark:bg-blue-900/30 hover:bg-blue-100 dark:hover:bg-blue-900/50" : "text-gray-400 dark:text-gray-500 hover:bg-gray-200/60 dark:hover:bg-gray-700/60 hover:text-gray-600 dark:hover:text-gray-300"}`}
                title={showRaw ? "Show preview" : "Show markdown"}
              >
                <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="currentColor">
                  <path d="M2.5 3A1.5 1.5 0 0 0 1 4.5v7A1.5 1.5 0 0 0 2.5 13h11a1.5 1.5 0 0 0 1.5-1.5v-7A1.5 1.5 0 0 0 13.5 3h-11zM3 9V7l1.5 2L6 7v2h1V5H6L4.5 7.5 3 5H2v4h1zm7-1h1.5L9.5 11V8H8V5h1v3z" />
                </svg>
              </button>
            )}
            <button
              onClick={refreshing ? () => {
                invoke("cancel_request").catch(() => {});
                invoke("log_action", { action: "Cancel clicked during refresh" }).catch(() => {});
                setRefreshing(false);
                refreshingRef.current = false;
              } : handleRefresh}
              className={`group flex items-center justify-center w-7 h-7 rounded-lg transition-colors ${refreshing ? "bg-blue-50 dark:bg-blue-900/30 text-copilot-blue hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/30 cursor-pointer" : "text-gray-500 dark:text-gray-400 hover:bg-gray-200/60 dark:hover:bg-gray-700/60 hover:text-gray-700 dark:hover:text-gray-300"}`}
              title={refreshing ? "Cancel" : "Regenerate"}
            >
              {refreshing ? (
                <>
                  {/* Spinner — visible by default, hidden on hover */}
                  <div className="h-3.5 w-3.5 animate-spin rounded-full border-[1.5px] border-copilot-blue border-t-transparent group-hover:hidden" />
                  {/* Stop square — hidden by default, visible on hover */}
                  <svg className="w-3.5 h-3.5 hidden group-hover:block" viewBox="0 0 16 16" fill="currentColor">
                    <rect x="3" y="3" width="10" height="10" rx="1" />
                  </svg>
                </>
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
              className="flex items-center justify-center w-7 h-7 rounded-lg text-gray-500 dark:text-gray-400 hover:bg-gray-200/60 dark:hover:bg-gray-700/60 hover:text-gray-700 dark:hover:text-gray-300 transition-colors"
              title="Copy"
            >
              <svg className="w-3.5 h-3.5" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                <rect x="5" y="5" width="9" height="9" rx="1.5" />
                <path d="M11 5V3.5A1.5 1.5 0 0 0 9.5 2h-6A1.5 1.5 0 0 0 2 3.5v6A1.5 1.5 0 0 0 3.5 11H5" />
              </svg>
            </button>
            {/* Replace button — Write Mode only (in Read Mode, there's nothing to replace in-place) */}
            {!isReadMode && (
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
                  <div className="absolute bottom-full right-0 mb-1 bg-white dark:bg-gray-800 rounded-lg shadow-lg border border-gray-200 dark:border-gray-700 py-1 min-w-[180px] z-50">
                    <button
                      onClick={(e) => { e.stopPropagation(); switchReplaceMode("rendered"); }}
                      className={`w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 ${replaceMode === "rendered" ? "text-copilot-blue font-medium" : "text-gray-700 dark:text-gray-300"}`}
                    >
                      {replaceMode === "rendered" && <span>✓</span>}
                      <span className={replaceMode !== "rendered" ? "ml-5" : ""}>Rendered text</span>
                    </button>
                    <button
                      onClick={(e) => { e.stopPropagation(); switchReplaceMode("markdown"); }}
                      className={`w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 ${replaceMode === "markdown" ? "text-copilot-blue font-medium" : "text-gray-700 dark:text-gray-300"}`}
                    >
                      {replaceMode === "markdown" && <span>✓</span>}
                      <span className={replaceMode !== "markdown" ? "ml-5" : ""}>Markdown</span>
                    </button>
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
      </div>
      {/* Copy success toast */}
      {copyToast && (
        <div className="absolute top-3 left-1/2 -translate-x-1/2 px-3 py-1.5 bg-gray-800 dark:bg-gray-200 text-white dark:text-gray-800 text-xs font-medium rounded-lg shadow-lg animate-fade-in-out z-50">
          ✓ Copied
        </div>
      )}
    </div>
  );
};

export default Popup;
