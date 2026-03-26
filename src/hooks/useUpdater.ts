import { useState, useCallback, useEffect, useRef } from "react";
import { check } from "@tauri-apps/plugin-updater";

export type UpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "ready"
  | "upToDate"
  | "error";

interface UpdateState {
  status: UpdateStatus;
  version: string | null;
  notes: string | null;
  error: string | null;
  progress: number; // 0-100
}

export function useUpdater(autoCheckDelay = 10_000) {
  const [state, setState] = useState<UpdateState>({
    status: "idle",
    version: null,
    notes: null,
    error: null,
    progress: 0,
  });

  const updateRef = useRef<Awaited<ReturnType<typeof check>> | null>(null);
  const hasAutoChecked = useRef(false);

  const checkForUpdate = useCallback(async () => {
    setState((s) => ({ ...s, status: "checking", error: null }));
    try {
      const update = await check();
      if (update) {
        updateRef.current = update;
        setState({
          status: "available",
          version: update.version,
          notes: update.body || null,
          error: null,
          progress: 0,
        });
      } else {
        setState({
          status: "upToDate",
          version: null,
          notes: null,
          error: null,
          progress: 0,
        });
      }
    } catch (err) {
      setState((s) => ({
        ...s,
        status: "error",
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  }, []);

  const downloadAndInstall = useCallback(async () => {
    const update = updateRef.current;
    if (!update) return;

    setState((s) => ({ ...s, status: "downloading", progress: 0 }));
    try {
      let downloaded = 0;
      let contentLength = 0;
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            contentLength = event.data.contentLength || 0;
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            if (contentLength > 0) {
              setState((s) => ({
                ...s,
                progress: Math.round((downloaded / contentLength) * 100),
              }));
            }
            break;
          case "Finished":
            break;
        }
      });
      setState((s) => ({ ...s, status: "ready", progress: 100 }));
      // The installer will handle restart on Windows (passive mode)
    } catch (err) {
      setState((s) => ({
        ...s,
        status: "error",
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  }, []);

  // Auto-check on mount (with delay)
  useEffect(() => {
    if (hasAutoChecked.current) return;
    hasAutoChecked.current = true;
    const timer = setTimeout(checkForUpdate, autoCheckDelay);
    return () => clearTimeout(timer);
  }, [autoCheckDelay, checkForUpdate]);

  return { ...state, checkForUpdate, downloadAndInstall };
}
