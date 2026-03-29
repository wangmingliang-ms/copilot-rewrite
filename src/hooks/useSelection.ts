import { useState, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

/** Matches the Rust SelectionInfo struct */
export interface SelectionInfo {
  text: string;
  mouse_x: number;
  mouse_y: number;
  source: "UIA" | "Clipboard";
  /** Whether the selection is from an input/editable element (Write Mode).
   *  false = non-input element (Read Mode) */
  is_input_element: boolean;
}

/** Matches the Rust RewriteAction enum */
export type RewriteAction = "Translate" | "Polish" | "TranslateAndPolish" | "ReadModeTranslate";

/** Matches the Rust ProcessResponse struct */
export interface ProcessResponse {
  original: string;
  result: string;
  action: RewriteAction;
}

/**
 * Hook that listens for text selection events emitted by the Rust backend.
 * The backend emits "selection-detected" events when the UIA engine
 * detects a new text selection and the debounce timer has elapsed.
 */
export function useSelection() {
  const [selection, setSelection] = useState<SelectionInfo | null>(null);

  useEffect(() => {
    // Listen for selection events from the Rust backend
    const unlisten = listen<SelectionInfo>("selection-detected", (event) => {
      setSelection(event.payload);
    });

    // Listen for selection-cleared events
    const unlistenClear = listen("selection-cleared", () => {
      setSelection(null);
    });

    return () => {
      unlisten.then((fn) => fn());
      unlistenClear.then((fn) => fn());
    };
  }, []);

  return { selection };
}
