import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

type Theme = "system" | "light" | "dark";
type ResolvedTheme = "light" | "dark";

/**
 * Hook that manages theme state.
 * - Reads `theme` from backend settings ("system" | "light" | "dark")
 * - When "system", detects OS dark mode via backend + matchMedia listener
 * - Applies/removes "dark" class on <html>
 */
export function useTheme() {
  const [theme, setTheme] = useState<Theme>("system");
  const [resolved, setResolved] = useState<ResolvedTheme>("light");

  // Resolve effective theme
  const resolve = useCallback(async (t: Theme) => {
    if (t === "light" || t === "dark") {
      setResolved(t);
      return;
    }
    // "system" — detect OS preference
    try {
      const sys = await invoke<string>("get_system_theme");
      setResolved(sys === "dark" ? "dark" : "light");
    } catch {
      // Fallback to matchMedia
      setResolved(window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
    }
  }, []);

  // Load initial theme from settings
  useEffect(() => {
    invoke<{ theme?: string }>("get_settings")
      .then((s) => {
        const t = (s.theme || "system") as Theme;
        setTheme(t);
        resolve(t);
      })
      .catch(() => resolve("system"));
  }, [resolve]);

  // Listen for OS theme changes (when in "system" mode)
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = (e: MediaQueryListEvent) => {
      if (theme === "system") {
        setResolved(e.matches ? "dark" : "light");
      }
    };
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, [theme]);

  // Apply dark class to <html>
  useEffect(() => {
    if (resolved === "dark") {
      document.documentElement.classList.add("dark");
    } else {
      document.documentElement.classList.remove("dark");
    }
  }, [resolved]);

  // Change theme
  const changeTheme = useCallback(async (newTheme: Theme) => {
    setTheme(newTheme);
    await resolve(newTheme);
    // Persist
    try {
      const s = await invoke<Record<string, unknown>>("get_settings");
      await invoke("update_settings", { settings: { ...s, theme: newTheme } });
    } catch { /* non-critical */ }
  }, [resolve]);

  return { theme, resolved, changeTheme };
}
