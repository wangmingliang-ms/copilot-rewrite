import { useState, useCallback, useEffect, useMemo, useRef, type FC } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { marked } from "marked";
import "github-markdown-css/github-markdown-light.css";
import iconImg from "../assets/icon-48.png";
import { SelectionInfo, ProcessResponse } from "../hooks/useSelection";
import { extractJsonStringValue, stripCodeFences, extractVocabulary, parseReadModeSeparator } from "../utils/jsonParser";
import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import * as Select from "@radix-ui/react-select";
import { Square, RefreshCw, ChevronDown, ChevronLeft, ChevronRight, Check, Sparkles, X, Settings, Copy, FileText, AlertCircle, Code } from "lucide-react";

// ── State machine ──
// icon (48×48) → loading (expanded, spinner) → streaming (expanded, text flowing) → expanded (final) | error
type PopupState = "icon" | "loading" | "streaming" | "expanded" | "error";

// Write Mode actions
type WriteAction = "TranslateAndPolish" | "Translate" | "Polish";
const WRITE_ACTIONS: { value: WriteAction; label: string }[] = [
  { value: "TranslateAndPolish", label: "Translate + Polish" },
  { value: "Translate", label: "Only Translate" },
  { value: "Polish", label: "Only Polish" },
];

// Read Mode actions
type ReadAction = "translate_summarize" | "simple_translate";
const READ_ACTIONS: { value: ReadAction; label: string }[] = [
  { value: "translate_summarize", label: "Summarize + Translate" },
  { value: "simple_translate", label: "Only Translate" },
];

interface CopilotModel {
  id: string;
  name: string;
  vendor: string;
  preview: boolean;
  category: string;
}

interface PopupProps {
  selection: SelectionInfo | null;
}

const Popup: FC<PopupProps> = ({ selection }) => {
  const [state, setState] = useState<PopupState>("icon");
  const stateRef = useRef<PopupState>("icon");
  const [streamingText, setStreamingText] = useState<string | null>(null);
  const [result, setResult] = useState<ProcessResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showRaw, setShowRaw] = useState(false);
  const [currentModel, setCurrentModel] = useState<string>("");
  const [currentModelId, setCurrentModelId] = useState<string>("");
  const [models, setModels] = useState<CopilotModel[]>([]);
  const [creativeMode, setCreativeMode] = useState<boolean>(false);
  const [replaceMode, setReplaceMode] = useState<"rendered" | "markdown">("rendered");
  const [showReplaceMenu, setShowReplaceMenu] = useState(false);

  // Result history for pagination
  const [history, setHistory] = useState<{ result: ProcessResponse }[]>([]);
  const [historyIndex, setHistoryIndex] = useState(-1);

  // Read Mode state
  const [isReadMode, setIsReadMode] = useState(false);
  const [readTab, setReadTab] = useState<"summary" | "translation" | "vocabulary">("summary");
  const [writeTab, setWriteTab] = useState<"translated" | "polished">("translated");

  // Top toolbar state — action selection
  const [currentWriteAction, setCurrentWriteAction] = useState<WriteAction>("TranslateAndPolish");
  const [currentReadAction, setCurrentReadAction] = useState<ReadAction>("translate_summarize");
  const [showActionMenu, setShowActionMenu] = useState(false);

  // Derived: is generating
  const isGenerating = state === "loading" || state === "streaming";

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

  // Refresh settings (model name + creative mode + theme + read mode + write action) from backend
  const refreshSettings = useCallback(async () => {
    try {
      const s = await invoke<{
        model: string; creative_mode: boolean; replace_mode: string; theme?: string;
        native_language?: string; target_language?: string; read_mode_enabled?: boolean; read_mode_sub?: string;
        write_action?: string;
      }>("get_settings");
      setCreativeMode(s.creative_mode || false);
      applyTheme(s.theme);
      setReplaceMode((s.replace_mode === "markdown" ? "markdown" : "rendered") as "rendered" | "markdown");
      // Restore write action
      const wa = s.write_action || "TranslateAndPolish";
      if (wa === "TranslateAndPolish" || wa === "Translate" || wa === "Polish") {
        setCurrentWriteAction(wa as WriteAction);
      }
      const rms = {
        native_language: s.native_language || "Chinese (Simplified)",
        target_language: s.target_language || "English",
        read_mode_sub: s.read_mode_sub || "translate_summarize",
      };
      setReadModeSettings(rms);
      setCurrentReadAction(rms.read_mode_sub === "simple_translate" ? "simple_translate" : "translate_summarize");
      if (!s.model) { setCurrentModel(""); setCurrentModelId(""); return; }
      setCurrentModelId(s.model);
      try {
        const fetchedModels = await invoke<CopilotModel[]>("list_models");
        setModels(fetchedModels);
        const match = fetchedModels.find((m) => m.id === s.model);
        setCurrentModel(match ? match.name : s.model);
      } catch {
        setCurrentModel(s.model);
      }
    } catch { /* ignore */ }
  }, []);

  // Listen for backend events
  useEffect(() => {
    const unResult = listen<ProcessResponse>("show-preview-result", (event) => {
      setStreamingText(null);
      setResult(event.payload);
      setError(null);
      setPopupState("expanded");
      setRefreshing(false);
      refreshingRef.current = false;
      // Save to history
      setHistory(prev => {
        const next = [...prev, { result: event.payload }];
        setHistoryIndex(next.length - 1);
        return next;
      });
    });

    const unLoading = listen("show-preview-loading", () => {
      setPopupState("loading");
      setError(null);
      setResult(null);
      setStreamingText(null);
    });

    // Listen for incremental streaming chunks
    const unChunk = listen<string>("show-preview-chunk", (event) => {
      setStreamingText(event.payload);
      // Transition from loading to streaming on first chunk
      if (stateRef.current === "loading") {
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
      setHistory([]);
      setHistoryIndex(-1);
      refreshSettings();
    });

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

  // ── Parse result ──
  const { reorganized, translated, readLayout, readTranslation, readSummary, readVocabulary } = useMemo(() => {
    const empty = { reorganized: "", translated: "", readLayout: "" as string, readTranslation: "", readSummary: "", readVocabulary: [] as { term: string; meaning: string }[] };
    if (!result?.result) return empty;
    const text = result.result.trim();

    // Write Mode: separator format
    const SEP = "---TRANSLATED---";
    const sepIdx = text.indexOf(SEP);
    if (sepIdx !== -1) {
      return { ...empty, reorganized: text.slice(0, sepIdx).trim(), translated: text.slice(sepIdx + SEP.length).trim() };
    }

    // Read Mode: separator format
    if (isReadMode) {
      const parsed = parseReadModeSeparator(text);
      return { ...empty, ...parsed };
    }

    // Legacy fallback: JSON parse
    try {
      const cleaned = stripCodeFences(text);
      let parsed: Record<string, unknown> | null = null;
      try {
        parsed = JSON.parse(cleaned);
      } catch {
        let fixed = "";
        let inString = false;
        for (let i = 0; i < cleaned.length; i++) {
          const ch = cleaned[i];
          if (ch === '\\' && inString) { fixed += ch + (cleaned[i + 1] || ""); i++; continue; }
          if (ch === '"') { inString = !inString; fixed += ch; continue; }
          if ((ch === '\n' || ch === '\r') && inString) { if (ch === '\r' && cleaned[i + 1] === '\n') i++; fixed += "\\n"; continue; }
          fixed += ch;
        }
        try { parsed = JSON.parse(fixed); } catch {
          const translation = extractJsonStringValue(cleaned, "translation");
          if (translation) {
            const summary = extractJsonStringValue(cleaned, "summary") || "";
            const vocabulary = extractVocabulary(cleaned);
            const layout = summary.trim() ? "withSummary" : vocabulary.length > 0 ? "withVocab" : "simple";
            return { ...empty, readLayout: layout, readTranslation: translation, readSummary: summary, readVocabulary: vocabulary };
          }
        }
      }
      if (parsed) {
        if (parsed.translation) {
          const hasVocab = Array.isArray(parsed.vocabulary) && parsed.vocabulary.length > 0;
          const hasSummary = typeof parsed.summary === "string" && parsed.summary.trim().length > 0;
          const layout = hasSummary ? "withSummary" : hasVocab ? "withVocab" : "simple";
          return { ...empty, readLayout: layout, readTranslation: String(parsed.translation), readSummary: hasSummary ? String(parsed.summary) : "", readVocabulary: hasVocab ? (parsed.vocabulary as { term: string; meaning: string }[]) : [] };
        }
        if (parsed.mode) {
          const hasSummary = typeof parsed.summary === "string" && parsed.summary.trim().length > 0;
          const hasVocab = Array.isArray(parsed.vocabulary) && parsed.vocabulary.length > 0;
          const layout = hasSummary ? "withSummary" : hasVocab ? "withVocab" : "simple";
          return { ...empty, readLayout: layout, readTranslation: String(parsed.translation || ""), readSummary: hasSummary ? String(parsed.summary) : "", readVocabulary: hasVocab ? (parsed.vocabulary as { term: string; meaning: string }[]) : [] };
        }
        if (parsed.reorganized && parsed.translated) {
          return { ...empty, reorganized: String(parsed.reorganized), translated: String(parsed.translated) };
        }
      }
    } catch {
      if (!isReadMode) {
        const dividerMatch = text.match(/\n---\n/);
        if (dividerMatch && dividerMatch.index !== undefined) {
          return { ...empty, reorganized: text.slice(0, dividerMatch.index).trim(), translated: text.slice(dividerMatch.index + dividerMatch[0].length).trim() };
        }
      }
    }
    const fallbackText = stripCodeFences(text);
    if (isReadMode) return { ...empty, readLayout: "simple", readTranslation: fallbackText };
    return { ...empty, translated: fallbackText };
  }, [result?.result, isReadMode]);

  // Render markdown
  const reorganizedHtml = useMemo(() => { if (!reorganized) return ""; marked.setOptions({ breaks: true, gfm: true }); return marked.parse(reorganized) as string; }, [reorganized]);
  const translatedHtml = useMemo(() => { if (!translated) return ""; marked.setOptions({ breaks: true, gfm: true }); return marked.parse(translated) as string; }, [translated]);
  const readSummaryHtml = useMemo(() => { if (!readSummary) return ""; marked.setOptions({ breaks: true, gfm: true }); return marked.parse(readSummary) as string; }, [readSummary]);
  const readTranslationHtml = useMemo(() => { if (!readTranslation) return ""; marked.setOptions({ breaks: true, gfm: true }); return marked.parse(readTranslation) as string; }, [readTranslation]);

  // Streaming parse
  const streamingParsed = useMemo(() => {
    const empty = { text: "", vocabulary: [] as { term: string; meaning: string }[], phase: "translated" as "reorganized" | "translated" };
    if (!streamingText) return empty;
    const raw = streamingText.trim();
    const SEP = "---TRANSLATED---";
    const sepIdx = raw.indexOf(SEP);
    if (sepIdx !== -1) return { text: raw.slice(sepIdx + SEP.length).trim() || "...", vocabulary: [], phase: "translated" as const };
    if (isReadMode) {
      const parsed = parseReadModeSeparator(raw);
      return { text: parsed.readTranslation || raw, vocabulary: parsed.readVocabulary, phase: "translated" as const };
    }
    const stripped = stripCodeFences(raw);
    if (stripped.startsWith("{")) {
      const keys = ["translation", "summary"];
      let bestContent = "";
      for (const key of keys) { const extracted = extractJsonStringValue(stripped, key); if (extracted !== null) bestContent = extracted; }
      const vocabulary = extractVocabulary(stripped);
      return { text: bestContent || stripped, vocabulary, phase: "translated" as const };
    }
    return { text: raw, vocabulary: [], phase: "reorganized" as const };
  }, [streamingText]);

  const streamingHtml = useMemo(() => {
    if (!streamingParsed.text) return "";
    marked.setOptions({ breaks: true, gfm: true });
    return marked.parse(streamingParsed.text) as string;
  }, [streamingParsed.text]);

  // Output text for Replace/Copy
  const effectiveReadTab = (() => {
    const hasSummary = readLayout === "withSummary" && !!readSummary;
    const hasVocab = readVocabulary.length > 0;
    if (hasSummary && readTab === "summary") return "summary";
    if (hasVocab && readTab === "vocabulary") return "vocabulary";
    return "translation";
  })();
  const effectiveWriteTab = reorganized && writeTab === "polished" ? "polished" : "translated";
  const outputText = isReadMode
    ? (effectiveReadTab === "summary" ? readSummary : effectiveReadTab === "vocabulary" ? readVocabulary.map(v => `${v.term} — ${v.meaning}`).join("\n") : readTranslation) || result?.result || ""
    : (effectiveWriteTab === "polished" ? reorganized : translated) || result?.result || "";
  const outputHtml = isReadMode
    ? (effectiveReadTab === "summary" ? readSummaryHtml : effectiveReadTab === "vocabulary" ? "" : readTranslationHtml)
    : (effectiveWriteTab === "polished" ? reorganizedHtml : translatedHtml);

  // Close menus on Escape
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (showReplaceMenu) { setShowReplaceMenu(false); return; }
        if (showActionMenu) { setShowActionMenu(false); return; }
        handleDismiss();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => { window.removeEventListener("keydown", handleKeyDown); };
  }, [showReplaceMenu, showActionMenu]);

  // Auto-dismiss on blur (skip when Radix menus are open — their Portal causes transient blur)
  useEffect(() => {
    if (state !== "expanded" && state !== "streaming" && state !== "loading" && state !== "error") return;
    const handleBlur = () => {
      setTimeout(async () => {
        if (showActionMenu || showReplaceMenu) return;
        if (!document.hasFocus()) handleDismiss();
      }, 100);
    };
    window.addEventListener("blur", handleBlur);
    return () => window.removeEventListener("blur", handleBlur);
  }, [state, showActionMenu, showReplaceMenu]);

  // Resize popup to fit content
  const contentRef = useRef<HTMLDivElement>(null);
  const hasResized = useRef(false);
  const sizeLockedForRegenerate = useRef(false);
  useEffect(() => {
    if ((state !== "expanded" && state !== "streaming" && state !== "loading") || !contentRef.current) return;
    if (sizeLockedForRegenerate.current) return;
    if (state === "expanded" && hasResized.current) return;
    const delay = state === "streaming" ? 200 : state === "loading" ? 150 : 50;
    const timer = setTimeout(() => {
      if (contentRef.current) {
        const card = contentRef.current;
        const oldMaxH = card.style.maxHeight;
        card.style.maxHeight = "none";
        const totalHeight = card.scrollHeight;
        card.style.maxHeight = oldMaxH;
        invoke("resize_popup_content", { height: Math.min(Math.max(totalHeight, 80), 400) }).catch(() => {});
        // For expanded state, do a second resize after 300ms to catch async markdown rendering
        if (state === "expanded") {
          setTimeout(() => {
            if (contentRef.current) {
              const c = contentRef.current;
              const old = c.style.maxHeight;
              c.style.maxHeight = "none";
              const h = c.scrollHeight;
              c.style.maxHeight = old;
              invoke("resize_popup_content", { height: Math.min(Math.max(h, 80), 400) }).catch(() => {});
            }
            hasResized.current = true;
            sizeLockedForRegenerate.current = true;
          }, 300);
        }
      }
    }, delay);
    return () => clearTimeout(timer);
  }, [state, streamingText, translatedHtml, reorganizedHtml, readSummaryHtml, readTranslationHtml, readLayout]);

  // ── Process invocation helper ──
  // Accept explicit readMode override to avoid stale closure issues
  // (setIsReadMode is async, so the closure may not reflect the latest value)
  const invokeProcess = useCallback(async (opts: {
    action?: WriteAction;
    readAction?: ReadAction;
    creative?: boolean;
    isRefresh?: boolean;
    readMode?: boolean;
  } = {}) => {
    if (!selection) return;
    const writeAction = opts.action ?? currentWriteAction;
    const readAction = opts.readAction ?? currentReadAction;
    const creative = opts.creative ?? creativeMode;
    const isRefresh = opts.isRefresh ?? false;
    const effectiveReadMode = opts.readMode ?? isReadMode;

    try {
      if (effectiveReadMode) {
        const summarize = readAction === "translate_summarize";
        await invoke("process_and_show_preview", {
          request: {
            text: selection.text,
            action: "ReadModeTranslate",
            creative_mode: creative,
            is_refresh: isRefresh,
            read_target_language: readModeSettings.native_language,
            read_summarize: summarize,
          },
        });
      } else {
        await invoke("process_and_show_preview", {
          request: {
            text: selection.text,
            action: writeAction,
            creative_mode: creative,
            is_refresh: isRefresh,
          },
        });
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (msg.includes("cancelled")) return;
      setError(msg);
      setPopupState("error");
      setRefreshing(false);
      refreshingRef.current = false;
    }
  }, [selection, currentWriteAction, currentReadAction, creativeMode, isReadMode, readModeSettings]);

  // ── Icon click handler ──
  const handleIconClick = useCallback(async () => {
    setPopupState("loading");
    await refreshSettings();

    try {
      const auth = await invoke<{ logged_in: boolean }>("get_auth_status");
      if (!auth.logged_in) {
        invoke("log_action", { action: "Icon clicked — not logged in, opening Settings" }).catch(() => {});
        setPopupState("icon");
        await invoke("open_settings");
        return;
      }
    } catch {
      setError("Please login via tray → Settings");
      setPopupState("error");
      return;
    }

    if (!selection) { setPopupState("icon"); return; }

    const readMode = selection.is_input_element === false;
    setIsReadMode(readMode);
    invoke("log_action", { action: `Icon clicked — ${readMode ? "Read" : "Write"} Mode (${selection.text.length} chars)` }).catch(() => {});
    await invokeProcess({ readMode });
  }, [selection, refreshSettings, invokeProcess]);

  const [, setRefreshing] = useState(false);
  const [copyToast, setCopyToast] = useState(false);
  const refreshingRef = useRef(false);

  // ── History navigation ──
  const handleHistoryPrev = useCallback(() => {
    if (historyIndex <= 0) return;
    const newIdx = historyIndex - 1;
    setHistoryIndex(newIdx);
    setResult(history[newIdx].result);
  }, [historyIndex, history]);

  const handleHistoryNext = useCallback(() => {
    if (historyIndex >= history.length - 1) return;
    const newIdx = historyIndex + 1;
    setHistoryIndex(newIdx);
    setResult(history[newIdx].result);
  }, [historyIndex, history]);

  // ── Regenerate / Refresh ──
  const handleRegenerate = useCallback(async () => {
    if (!selection || isGenerating) return;
    invoke("log_action", { action: "Regenerate clicked" }).catch(() => {});
    setPopupState("loading");
    setError(null);
    setResult(null);
    setStreamingText(null);
    await invokeProcess({ isRefresh: true });
  }, [selection, isGenerating, invokeProcess]);

  // ── Stop generating ──
  const handleStop = useCallback(() => {
    invoke("cancel_request").catch(() => {});
    invoke("log_action", { action: "Stop generating clicked" }).catch(() => {});
  }, []);

  // ── Model change → persist + auto-regenerate ──
  const handleModelChange = useCallback(async (modelId: string) => {
    if (!modelId || modelId === currentModelId) return;
    setCurrentModelId(modelId);
    const match = models.find((m) => m.id === modelId);
    setCurrentModel(match ? match.name : modelId);
    invoke("log_action", { action: `Model changed to: ${modelId}` }).catch(() => {});

    // Persist
    try {
      const s = await invoke<Record<string, unknown>>("get_settings");
      await invoke("update_settings", { settings: { ...s, model: modelId } });
    } catch { /* non-critical */ }

    // Auto-regenerate if we have a result
    if (state === "expanded" || state === "error") {
      await invoke("cancel_request").catch(() => {});
      setPopupState("loading");
      setStreamingText(null);
      setResult(null);
      setRefreshing(true);
      refreshingRef.current = true;
      await invokeProcess({ isRefresh: true });
    }
  }, [currentModelId, models, state, invokeProcess]);

  // ── Action dropdown change → persist + auto-regenerate ──
  const handleWriteActionChange = useCallback(async (action: WriteAction) => {
    if (action === currentWriteAction) return;
    setCurrentWriteAction(action);
    setShowActionMenu(false);
    invoke("log_action", { action: `Write action changed to: ${action}` }).catch(() => {});

    // Persist to settings
    try {
      const s = await invoke<Record<string, unknown>>("get_settings");
      await invoke("update_settings", { settings: { ...s, write_action: action } });
    } catch { /* non-critical */ }

    // Auto-regenerate
    await invoke("cancel_request").catch(() => {});
    setPopupState("loading");
    setStreamingText(null);
    setResult(null);
    setRefreshing(true);
    refreshingRef.current = true;
    await invokeProcess({ action, isRefresh: true });
  }, [currentWriteAction, invokeProcess]);

  const handleReadActionChange = useCallback(async (action: ReadAction) => {
    if (action === currentReadAction) return;
    setCurrentReadAction(action);
    setShowActionMenu(false);
    invoke("log_action", { action: `Read action changed to: ${action}` }).catch(() => {});

    // Persist to settings
    try {
      const s = await invoke<Record<string, unknown>>("get_settings");
      await invoke("update_settings", { settings: { ...s, read_mode_sub: action } });
    } catch { /* non-critical */ }

    await invoke("cancel_request").catch(() => {});
    setPopupState("loading");
    setStreamingText(null);
    setResult(null);
    setRefreshing(true);
    refreshingRef.current = true;
    await invokeProcess({ readAction: action, isRefresh: true });
  }, [currentReadAction, invokeProcess]);

  // ── Creative mode toggle → persist + auto-regenerate ──
  const handleCreativeToggle = useCallback(async () => {
    const newCreative = !creativeMode;
    setCreativeMode(newCreative);
    invoke("log_action", { action: `Creative mode toggled: ${newCreative}` }).catch(() => {});

    // Persist
    try {
      const s = await invoke<Record<string, unknown>>("get_settings");
      await invoke("update_settings", { settings: { ...s, creative_mode: newCreative } });
    } catch { /* non-critical */ }

    // Auto-regenerate
    await invoke("cancel_request").catch(() => {});
    setPopupState("loading");
    setStreamingText(null);
    setResult(null);
    setRefreshing(true);
    refreshingRef.current = true;
    await invokeProcess({ creative: newCreative, isRefresh: true });
  }, [creativeMode, invokeProcess]);

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
    if (stateRef.current === "streaming" || stateRef.current === "loading") {
      invoke("cancel_request").catch(() => {});
    }
    try { await invoke("dismiss_popup"); } catch {}
    resetState();
  }, []);

  const resetState = () => {
    setPopupState("icon");
    setResult(null);
    setError(null);
    setStreamingText(null);
    setShowRaw(false);
    setRefreshing(false);
    refreshingRef.current = false;
    hasResized.current = false;
    sizeLockedForRegenerate.current = false;
    setIsReadMode(false);
    setReadTab("summary");
    setWriteTab("translated");
    setHistory([]);
    setHistoryIndex(-1);
    setShowActionMenu(false);
    setShowReplaceMenu(false);
  };

  // ── Shared card style — elevated card with border + layered shadows ──
  const cardStyle = {
    background: isDark ? "#1e293b" : "#ffffff",
    border: isDark ? "1px solid rgba(255,255,255,0.12)" : "1px solid #d1d5db",
    borderRadius: "12px",
    boxShadow: isDark
      ? "0 1px 3px rgba(0,0,0,0.4), 0 6px 16px rgba(0,0,0,0.35), 0 16px 48px rgba(0,0,0,0.45)"
      : "0 1px 3px rgba(0,0,0,0.08), 0 8px 24px rgba(0,0,0,0.12), 0 24px 48px rgba(0,0,0,0.08)",
  };

  // Inner content card style
  const contentCardClass = "border border-gray-200 dark:border-gray-600 rounded-lg shadow-[inset_0_1px_3px_rgba(0,0,0,0.06)] dark:shadow-[inset_0_1px_3px_rgba(0,0,0,0.2)]";

  // ══════════════════════════════════════════════════════════
  // ── Top Toolbar ──
  // ══════════════════════════════════════════════════════════
  // Shared button style for toolbar buttons — bordered, subtle shadow
  const toolbarBtnClass = "border border-gray-200 dark:border-gray-600 shadow-sm";

  const renderTopToolbar = () => {
    const disabledClass = isGenerating ? "opacity-50 pointer-events-none" : "";

    return (
      <div
        className="flex-shrink-0 flex items-center gap-1.5 px-4 py-2.5 border-b border-gray-200 dark:border-gray-600"
        style={{ background: isDark ? "rgba(15,23,42,0.6)" : "rgba(249,250,251,0.8)" }}
      >
        {/* Action dropdown (includes creative mode toggle for Write Mode) */}
        <div className={`${disabledClass}`}>
          <DropdownMenu.Root open={showActionMenu} onOpenChange={setShowActionMenu}>
            <DropdownMenu.Trigger asChild>
              <button
                className={`flex items-center gap-1 px-2.5 py-1 rounded-md text-xs font-medium text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700/60 transition-colors ${toolbarBtnClass}`}
                disabled={isGenerating}
              >
                {!isReadMode && creativeMode && <Sparkles size={10} className="text-blue-500 flex-shrink-0" />}
                {isReadMode
                  ? READ_ACTIONS.find((a) => a.value === currentReadAction)?.label
                  : WRITE_ACTIONS.find((a) => a.value === currentWriteAction)?.label}
                <ChevronDown size={10} className="ml-0.5" />
              </button>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content
                className="bg-white dark:bg-gray-800 rounded-lg shadow-lg border border-gray-200 dark:border-gray-700 py-1 min-w-[180px] z-50"
                sideOffset={4}
                align="start"
              >
                {isReadMode
                  ? READ_ACTIONS.map((a) => (
                      <DropdownMenu.CheckboxItem
                        key={a.value}
                        checked={currentReadAction === a.value}
                        onCheckedChange={() => handleReadActionChange(a.value)}
                        className={`w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 outline-none cursor-pointer ${currentReadAction === a.value ? "text-copilot-blue font-medium" : "text-gray-700 dark:text-gray-300"}`}
                      >
                        <DropdownMenu.ItemIndicator className="w-3 inline-flex justify-center flex-shrink-0">
                          <Check size={10} />
                        </DropdownMenu.ItemIndicator>
                        <span className={currentReadAction !== a.value ? "ml-5" : ""}>{a.label}</span>
                      </DropdownMenu.CheckboxItem>
                    ))
                  : <>
                      {WRITE_ACTIONS.map((a) => (
                        <DropdownMenu.CheckboxItem
                          key={a.value}
                          checked={currentWriteAction === a.value}
                          onCheckedChange={() => handleWriteActionChange(a.value)}
                          className={`w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 outline-none cursor-pointer ${currentWriteAction === a.value ? "text-copilot-blue font-medium" : "text-gray-700 dark:text-gray-300"}`}
                        >
                          <DropdownMenu.ItemIndicator className="w-3 inline-flex justify-center flex-shrink-0">
                            <Check size={10} />
                          </DropdownMenu.ItemIndicator>
                          <span className={currentWriteAction !== a.value ? "ml-5" : ""}>{a.label}</span>
                        </DropdownMenu.CheckboxItem>
                      ))}
                      <DropdownMenu.Separator className="h-px bg-gray-200 dark:bg-gray-700 my-1" />
                      <DropdownMenu.CheckboxItem
                        checked={creativeMode}
                        onCheckedChange={() => handleCreativeToggle()}
                        className={`w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 outline-none cursor-pointer ${creativeMode ? "text-blue-600 dark:text-blue-400 font-medium" : "text-gray-700 dark:text-gray-300"}`}
                      >
                        <DropdownMenu.ItemIndicator className="w-3 inline-flex justify-center flex-shrink-0">
                          <Check size={10} />
                        </DropdownMenu.ItemIndicator>
                        <span className={`flex items-center gap-1.5 ${!creativeMode ? "ml-5" : ""}`}>
                          More Creative
                          <Sparkles size={12} className="flex-shrink-0" />
                        </span>
                      </DropdownMenu.CheckboxItem>
                    </>
                }
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
        </div>

        {/* Model dropdown */}
        <div className={`${disabledClass}`}>
          <Select.Root value={currentModelId} onValueChange={handleModelChange}>
            <Select.Trigger
              className={`flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-mono text-gray-500 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700/60 transition-colors max-w-[150px] ${toolbarBtnClass}`}
              disabled={isGenerating}
            >
              <Select.Value placeholder="Select model">
                {currentModel || currentModelId || "No model"}
              </Select.Value>
              <Select.Icon>
                <ChevronDown size={10} className="flex-shrink-0" />
              </Select.Icon>
            </Select.Trigger>
            <Select.Portal>
              <Select.Content
                className="bg-white dark:bg-gray-800 rounded-lg shadow-lg border border-gray-200 dark:border-gray-700 overflow-hidden z-50"
                position="popper"
                sideOffset={4}
              >
                <Select.Viewport className="p-1 max-h-[300px]">
                  {(() => {
                    const grouped = models.reduce<Record<string, CopilotModel[]>>((acc, m) => {
                      const vendor = m.vendor || "Other";
                      (acc[vendor] = acc[vendor] || []).push(m);
                      return acc;
                    }, {});
                    const sortedVendors = Object.keys(grouped).sort();
                    return sortedVendors.map((vendor) => (
                      <Select.Group key={vendor}>
                        <Select.Label className="px-2.5 py-1 text-[10px] font-semibold text-gray-400 dark:text-gray-500 uppercase">{vendor}</Select.Label>
                        {grouped[vendor].sort((a, b) => a.name.localeCompare(b.name)).map((model) => (
                          <Select.Item
                            key={model.id}
                            value={model.id}
                            className="px-2.5 py-1.5 text-xs text-gray-900 dark:text-gray-100 rounded cursor-pointer outline-none data-[highlighted]:bg-gray-100 dark:data-[highlighted]:bg-gray-700 flex items-center gap-2"
                          >
                            <Select.ItemText>
                              {model.name}{model.preview ? " (Preview)" : ""}{model.category === "powerful" ? " ⚡" : ""}
                            </Select.ItemText>
                            <Select.ItemIndicator>
                              <Check size={12} className="text-copilot-blue" />
                            </Select.ItemIndicator>
                          </Select.Item>
                        ))}
                      </Select.Group>
                    ));
                  })()}
                  {models.length === 0 && currentModelId && (
                    <Select.Item value={currentModelId} className="px-2.5 py-1.5 text-xs text-gray-900 dark:text-gray-100">
                      <Select.ItemText>{currentModel || currentModelId}</Select.ItemText>
                    </Select.Item>
                  )}
                </Select.Viewport>
              </Select.Content>
            </Select.Portal>
          </Select.Root>
        </div>

        {/* Settings gear */}
        <button
          onClick={() => { invoke("log_action", { action: "Settings button clicked" }).catch(() => {}); invoke("open_settings").catch(() => {}); }}
          className={`flex items-center justify-center w-6 h-6 rounded-md text-gray-400 dark:text-gray-500 hover:bg-gray-200/60 dark:hover:bg-gray-700/60 hover:text-gray-600 dark:hover:text-gray-300 transition-colors border border-gray-200 dark:border-gray-600 shadow-sm ${disabledClass}`}
          title="Settings"
        >
          <Settings size={14} />
        </button>

        <div className="flex-1" />

        {/* Dismiss */}
        <button
          onClick={handleDismiss}
          className="flex items-center justify-center w-6 h-6 rounded-md text-gray-400 dark:text-gray-500 hover:bg-gray-200/60 dark:hover:bg-gray-700/60 hover:text-gray-600 dark:hover:text-gray-300 transition-colors"
          title="Dismiss"
        >
          <X size={14} />
        </button>
      </div>
    );
  };

  // ══════════════════════════════════════════════════════════
  // ── Bottom Action Bar ──
  // ══════════════════════════════════════════════════════════
  const renderBottomBar = () => {
    const disabledActions = isGenerating ? "opacity-50 pointer-events-none" : "";

    return (
      <div
        className="flex-shrink-0 flex items-center gap-1.5 border-t border-gray-200 dark:border-gray-600 px-4 py-2.5"
        style={{ background: isDark ? "rgba(15,23,42,0.8)" : "rgba(249,250,251,0.8)" }}
      >
        {/* Regenerate / Generating / Stop button */}
        {isGenerating ? (
          <button
            onClick={handleStop}
            className={`group flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-medium bg-blue-50 dark:bg-blue-900/30 text-copilot-blue hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/30 transition-colors ${toolbarBtnClass}`}
            title="Stop generating"
          >
            <div className="h-3.5 w-3.5 animate-spin rounded-full border-[1.5px] border-copilot-blue border-t-transparent group-hover:hidden" />
            <Square size={14} className="hidden group-hover:block" />
            <span className="group-hover:hidden">Generating...</span>
            <span className="hidden group-hover:inline">Stop</span>
          </button>
        ) : (
          <button
            onClick={handleRegenerate}
            className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-medium text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700/60 transition-colors ${toolbarBtnClass}`}
            title="Regenerate"
          >
            <RefreshCw size={14} />
            Regenerate
          </button>
        )}

        {/* History pagination */}
        {history.length > 1 && !isGenerating && (
          <div className="flex items-center gap-0 text-xs text-gray-500 dark:text-gray-400">
            <button
              onClick={handleHistoryPrev}
              disabled={historyIndex <= 0}
              className="flex items-center justify-center w-5 h-5 rounded hover:bg-gray-200/60 dark:hover:bg-gray-700/60 transition-colors disabled:opacity-30 disabled:pointer-events-none"
              title="Previous result"
            >
              <ChevronLeft size={14} />
            </button>
            <span className="tabular-nums text-[11px] select-none">
              {historyIndex + 1} of {history.length}
            </span>
            <button
              onClick={handleHistoryNext}
              disabled={historyIndex >= history.length - 1}
              className="flex items-center justify-center w-5 h-5 rounded hover:bg-gray-200/60 dark:hover:bg-gray-700/60 transition-colors disabled:opacity-30 disabled:pointer-events-none"
              title="Next result"
            >
              <ChevronRight size={14} />
            </button>
          </div>
        )}

        <div className="flex-1" />

        <div className={`flex items-center gap-1 ${disabledActions}`}>
          <button
            onClick={handleCopy}
            className="flex items-center gap-1 px-2 py-1 rounded-md text-xs font-medium text-gray-600 dark:text-gray-300 border border-gray-200 dark:border-gray-600 shadow-sm hover:bg-gray-100 dark:hover:bg-gray-700/60 transition-colors"
            title="Copy"
          >
            <Copy size={12} />
            Copy
          </button>

          {/* Replace split-button — Write Mode only */}
          {!isReadMode && (
            <div className="flex shadow-sm rounded-md">
              <button
                onClick={handleReplace}
                className="flex items-center gap-1.5 rounded-l-md bg-copilot-blue px-2.5 py-1 text-xs font-medium text-white hover:bg-copilot-blue-hover transition-colors"
                title={replaceMode === "rendered" ? "Replace with rendered text" : "Replace with markdown"}
              >
                {replaceMode === "rendered" ? (
                  <FileText size={12} />
                ) : (
                  <Code size={14} />
                )}
                Replace
              </button>
              <DropdownMenu.Root open={showReplaceMenu} onOpenChange={setShowReplaceMenu}>
                <DropdownMenu.Trigger asChild>
                  <button
                    className="flex items-center rounded-r-md bg-copilot-blue px-1.5 py-1 text-white hover:bg-copilot-blue-hover transition-colors border-l border-white/20"
                    title="Replace options"
                  >
                    <ChevronDown size={10} />
                  </button>
                </DropdownMenu.Trigger>
                <DropdownMenu.Portal>
                  <DropdownMenu.Content
                    className="bg-white dark:bg-gray-800 rounded-lg shadow-lg border border-gray-200 dark:border-gray-700 py-1 min-w-[180px] z-50"
                    sideOffset={4}
                    align="end"
                    side="top"
                  >
                    <DropdownMenu.CheckboxItem
                      checked={replaceMode === "rendered"}
                      onCheckedChange={() => switchReplaceMode("rendered")}
                      className={`w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 outline-none cursor-pointer ${replaceMode === "rendered" ? "text-copilot-blue font-medium" : "text-gray-700 dark:text-gray-300"}`}
                    >
                      <DropdownMenu.ItemIndicator className="w-3 inline-flex justify-center flex-shrink-0">
                        <Check size={10} />
                      </DropdownMenu.ItemIndicator>
                      <FileText size={14} className={`flex-shrink-0 ${replaceMode !== "rendered" ? "ml-5" : ""}`} />
                      Rendered text
                    </DropdownMenu.CheckboxItem>
                    <DropdownMenu.CheckboxItem
                      checked={replaceMode === "markdown"}
                      onCheckedChange={() => switchReplaceMode("markdown")}
                      className={`w-full text-left px-3 py-1.5 text-xs hover:bg-gray-100 dark:hover:bg-gray-700 flex items-center gap-2 outline-none cursor-pointer ${replaceMode === "markdown" ? "text-copilot-blue font-medium" : "text-gray-700 dark:text-gray-300"}`}
                    >
                      <DropdownMenu.ItemIndicator className="w-3 inline-flex justify-center flex-shrink-0">
                        <Check size={10} />
                      </DropdownMenu.ItemIndicator>
                      <Code size={14} className={`flex-shrink-0 ${replaceMode !== "markdown" ? "ml-5" : ""}`} />
                      Markdown
                    </DropdownMenu.CheckboxItem>
                  </DropdownMenu.Content>
                </DropdownMenu.Portal>
              </DropdownMenu.Root>
            </div>
          )}
        </div>
      </div>
    );
  };

  // ══════════════════════════════════════════════════════════
  // ── Render States ──
  // ══════════════════════════════════════════════════════════

  // ── Icon state (48×48) ──
  if (state === "icon") {
    return (
      <div className="w-screen h-screen flex items-center justify-center" style={{ padding: "20px", background: "transparent", pointerEvents: "none" }}>
        <div className="w-full h-full flex items-center justify-center"
          style={{
            pointerEvents: "auto",
            background: isDark
              ? "linear-gradient(135deg, #1e293b 0%, #0f172a 100%)"
              : "linear-gradient(135deg, #fff 0%, #f0f4ff 100%)",
            borderRadius: "50%",
            border: isDark ? "1px solid rgba(96,165,250,0.25)" : "1px solid rgba(0,120,212,0.15)",
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
            <img src={iconImg} alt="Translate" className="w-7 h-7 transition-transform group-hover:scale-110" draggable={false} />
          </button>
        </div>
      </div>
    );
  }

  // ── Loading state (expanded UI with centered spinner) ──
  if (state === "loading") {
    return (
      <div className="w-screen h-screen" style={{ padding: "20px", background: "transparent" }}>
        <div ref={contentRef} className="flex flex-col rounded-lg overflow-hidden h-full" style={cardStyle}>
          {renderTopToolbar()}
          <div className={`flex-1 flex items-center justify-center mx-4 my-2 ${contentCardClass}`}>
            <div className="flex flex-col items-center gap-3">
              <div className="h-6 w-6 animate-spin rounded-full border-2 border-copilot-blue border-t-transparent" />
              <span className="text-sm text-gray-400 dark:text-gray-500">Generating...</span>
            </div>
          </div>
          {renderBottomBar()}
        </div>
      </div>
    );
  }

  // ── Streaming state (expanded UI with flowing text) ──
  if (state === "streaming") {
    return (
      <div className="w-screen h-screen" style={{ padding: "20px", background: "transparent" }}>
        <div ref={contentRef} className="flex flex-col rounded-lg overflow-hidden h-full" style={cardStyle}>
          {renderTopToolbar()}
          <div className={`flex-1 min-h-0 overflow-auto px-5 pt-4 pb-3 mx-4 my-2 ${contentCardClass}`} style={{ userSelect: "text", WebkitUserSelect: "text" }}>
            {/* Phase indicator for "thinking" (reorganized section before separator) */}
            {streamingParsed.phase === "reorganized" && !isReadMode && (
              <div className="text-[10px] font-medium text-gray-400 dark:text-gray-500 uppercase tracking-wide mb-2 flex items-center gap-1.5">
                <div className="h-1.5 w-1.5 rounded-full bg-amber-400 animate-pulse" />
                Thinking...
              </div>
            )}
            <div
              className={`markdown-body text-[13.5px] leading-[1.7] streaming-content ${streamingParsed.phase === "reorganized" && !isReadMode ? "opacity-60" : ""}`}
              style={{ background: "transparent" }}
              dangerouslySetInnerHTML={{ __html: streamingHtml }}
            />
            {/* Vocabulary entries during streaming */}
            {streamingParsed.vocabulary.length > 0 && (
              <div className="space-y-2 border-t border-gray-100 dark:border-gray-700 pt-2 mt-3">
                <div className="text-[11px] font-medium text-gray-400 dark:text-gray-500 uppercase tracking-wide">Vocabulary</div>
                {streamingParsed.vocabulary.map((v, i) => (
                  <div key={i} className="pl-3 border-l-2 border-purple-200 dark:border-purple-800">
                    <span className="text-[12.5px] font-semibold text-purple-600 dark:text-purple-400">{v.term}</span>
                    <span className="text-[12px] text-gray-600 dark:text-gray-400"> — {v.meaning}</span>
                  </div>
                ))}
              </div>
            )}
            {/* Blinking cursor indicator */}
            <span className="inline-block w-[2px] h-[1em] bg-copilot-blue align-text-bottom animate-pulse ml-0.5" />
          </div>
          {renderBottomBar()}
        </div>
      </div>
    );
  }

  // ── Error state ──
  if (state === "error") {
    return (
      <div className="w-screen h-screen" style={{ padding: "20px", background: "transparent" }}>
        <div className="flex flex-col rounded-lg h-full overflow-hidden" style={cardStyle}>
          {renderTopToolbar()}
          <div className={`flex-1 px-5 py-4 flex items-start gap-2.5 mx-4 my-2 ${contentCardClass}`}>
            <div className="flex-shrink-0 w-5 h-5 rounded-full bg-red-50 dark:bg-red-900/30 flex items-center justify-center mt-0.5">
              <AlertCircle size={12} className="text-red-500" />
            </div>
            <p className="text-[13px] leading-[1.5] text-red-600 dark:text-red-400">{error}</p>
          </div>
          {renderBottomBar()}
        </div>
      </div>
    );
  }

  // ── Expanded state (final result) ──
  return (
    <div className="w-screen h-screen" style={{ padding: "20px", background: "transparent" }}>
      <div ref={contentRef} className="flex flex-col rounded-lg overflow-hidden h-full" style={cardStyle}>
        {renderTopToolbar()}

        {isReadMode ? (
          /* ── Read Mode Content ── */
          (() => {
            const hasSummary = readLayout === "withSummary" && !!readSummary;
            const hasVocab = readVocabulary.length > 0;
            const tabCount = (hasSummary ? 1 : 0) + 1 /* translation always */ + (hasVocab ? 1 : 0);
            const activeTab = hasSummary && readTab === "summary" ? "summary"
              : hasVocab && readTab === "vocabulary" ? "vocabulary"
              : "translation";
            const tabBtnClass = (active: boolean) =>
              `text-[11px] uppercase tracking-wide py-2 px-3 transition-colors font-medium ${active
                ? "text-copilot-blue border-b-2 border-copilot-blue"
                : "text-gray-400 dark:text-gray-500 hover:text-gray-600 dark:hover:text-gray-300 border-b-2 border-transparent"}`;
            return (
              <>
                {/* Tab content */}
                <div className={`flex-1 min-h-0 overflow-auto px-5 pt-4 pb-3 mx-4 my-2 ${contentCardClass}`} style={{ userSelect: "text", WebkitUserSelect: "text" }}>
                  {activeTab === "summary" && (
                    showRaw ? (
                      <pre className="text-[12px] leading-[1.6] text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words font-mono">{readSummary}</pre>
                    ) : (
                      <div className="markdown-body text-[13.5px] leading-[1.7]" style={{ background: "transparent" }} dangerouslySetInnerHTML={{ __html: readSummaryHtml }} />
                    )
                  )}
                  {activeTab === "translation" && (
                    showRaw ? (
                      <pre className="text-[12px] leading-[1.6] text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words font-mono">{readTranslation}</pre>
                    ) : (
                      <div className="markdown-body text-[13.5px] leading-[1.7]" style={{ background: "transparent" }} dangerouslySetInnerHTML={{ __html: readTranslationHtml }} />
                    )
                  )}
                  {activeTab === "vocabulary" && (
                    <div className="space-y-2">
                      {readVocabulary.map((v: { term: string; meaning: string }, i: number) => (
                        <div key={i} className="pl-3 border-l-2 border-purple-200 dark:border-purple-800">
                          <span className="text-[12.5px] font-semibold text-purple-600 dark:text-purple-400">{v.term}</span>
                          <span className="text-[12px] text-gray-600 dark:text-gray-400"> — {v.meaning}</span>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
                {/* Tab bar — only shown when multiple tabs available */}
                {tabCount > 1 && (
                  <div className="flex-shrink-0 flex items-center gap-0 px-4 border-t border-gray-200 dark:border-gray-600">
                    {hasSummary && (
                      <button onClick={() => setReadTab("summary")} className={tabBtnClass(activeTab === "summary")}>
                        Summary
                      </button>
                    )}
                    <button onClick={() => setReadTab("translation")} className={tabBtnClass(activeTab === "translation")}>
                      Translation
                    </button>
                    {hasVocab && (
                      <button onClick={() => setReadTab("vocabulary")} className={tabBtnClass(activeTab === "vocabulary")}>
                        Vocabulary
                      </button>
                    )}
                  </div>
                )}
              </>
            );
          })()
        ) : (
          /* ── Write Mode Content ── */
          (() => {
            const hasPolished = !!reorganizedHtml;
            const tabCount = hasPolished ? 2 : 1;
            const activeTab = hasPolished && writeTab === "polished" ? "polished" : "translated";
            const tabBtnClass = (active: boolean) =>
              `text-[11px] uppercase tracking-wide py-2 px-3 transition-colors font-medium ${active
                ? "text-copilot-blue border-b-2 border-copilot-blue"
                : "text-gray-400 dark:text-gray-500 hover:text-gray-600 dark:hover:text-gray-300 border-b-2 border-transparent"}`;
            return (
              <>
                <div className={`flex-1 min-h-0 overflow-auto px-5 pt-4 pb-3 mx-4 my-2 ${contentCardClass}`} style={{ userSelect: "text", WebkitUserSelect: "text" }}>
                  {activeTab === "translated" && (
                    showRaw ? (
                      <pre className="text-[12px] leading-[1.6] text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words font-mono">{translated}</pre>
                    ) : (
                      <div className="markdown-body text-[13.5px] leading-[1.7]" style={{ background: "transparent" }} dangerouslySetInnerHTML={{ __html: translatedHtml }} />
                    )
                  )}
                  {activeTab === "polished" && (
                    showRaw ? (
                      <pre className="text-[12px] leading-[1.6] text-gray-700 dark:text-gray-300 whitespace-pre-wrap break-words font-mono">{reorganized}</pre>
                    ) : (
                      <div className="markdown-body text-[13.5px] leading-[1.7]" style={{ background: "transparent" }} dangerouslySetInnerHTML={{ __html: reorganizedHtml }} />
                    )
                  )}
                </div>
                {tabCount > 1 && (
                  <div className="flex-shrink-0 flex items-center gap-0 px-4 border-t border-gray-200 dark:border-gray-600">
                    <button onClick={() => setWriteTab("translated")} className={tabBtnClass(activeTab === "translated")}>
                      Translated
                    </button>
                    <button onClick={() => setWriteTab("polished")} className={tabBtnClass(activeTab === "polished")}>
                      Polished
                    </button>
                  </div>
                )}
              </>
            );
          })()
        )}

        {renderBottomBar()}
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
