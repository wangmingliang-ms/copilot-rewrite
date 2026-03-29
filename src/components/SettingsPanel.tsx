import { useState, useEffect, useCallback, type FC } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-shell";
import { getVersion } from "@tauri-apps/api/app";
import { useUpdater } from "../hooks/useUpdater";

type Theme = "system" | "light" | "dark";

interface ThemeCtx {
  theme: Theme;
  resolved: "light" | "dark";
  changeTheme: (t: Theme) => Promise<void>;
}

interface AuthStatus {
  logged_in: boolean;
  username: string | null;
}

interface DeviceCodeInfo {
  user_code: string;
  verification_uri: string;
}

interface Settings {
  target_language: string;
  auto_start: boolean;
  poll_interval_ms: number;
  beast_mode: boolean;
  model: string;
  theme: string;
  native_language: string;
  read_mode_enabled: boolean;
  read_mode_sub: string;
}

const LANGUAGES = [
  "English", "Chinese (Simplified)", "Chinese (Traditional)",
  "Japanese", "Korean", "French", "German", "Spanish",
  "Portuguese", "Russian", "Arabic", "Hindi", "Italian",
];

interface CopilotModel {
  id: string;
  name: string;
  version: string;
  vendor: string;
  preview: boolean;
  category: string;
}

const SettingsPanel: FC<{ themeCtx: ThemeCtx }> = ({ themeCtx }) => {
  const [authStatus, setAuthStatus] = useState<AuthStatus>({ logged_in: false, username: null });
  const [settings, setSettings] = useState<Settings>({
    target_language: "English",
    auto_start: false,
    poll_interval_ms: 100,
    beast_mode: false,
    model: "claude-sonnet-4",
    theme: "system",
    native_language: "Chinese (Simplified)",
    read_mode_enabled: true,
    read_mode_sub: "translate_summarize",
  });
  const [loginStep, setLoginStep] = useState<"idle" | "loading" | "code" | "waiting" | "error">("idle");
  const [deviceCode, setDeviceCode] = useState<DeviceCodeInfo | null>(null);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [saved, setSaved] = useState(false);
  const [models, setModels] = useState<CopilotModel[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [appVersion, setAppVersion] = useState("0.0.0");
  const updater = useUpdater(5_000); // auto-check 5s after settings opens

  // Load on mount
  const [initialLoaded, setInitialLoaded] = useState(false);
  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
    invoke<AuthStatus>("get_auth_status").then(setAuthStatus).catch(() => {});
    invoke<Settings>("get_settings").then((s) => {
      setSettings(s);
      setInitialLoaded(true);
    }).catch(() => { setInitialLoaded(true); });
    fetchModels();
  }, []);

  // Auto-save settings on any change (skip initial load)
  useEffect(() => {
    if (!initialLoaded) return;
    const timer = setTimeout(() => {
      invoke("update_settings", { settings }).then(() => {
        invoke("log_action", { action: `Settings saved — model=${settings.model}, lang=${settings.target_language}, beast=${settings.beast_mode}, autoStart=${settings.auto_start}` }).catch(() => {});
        setSaved(true);
        setTimeout(() => setSaved(false), 1500);
      }).catch((err) => console.error("Auto-save failed:", err));
    }, 300);
    return () => clearTimeout(timer);
  }, [settings, initialLoaded]);

  const fetchModels = useCallback(async () => {
    setModelsLoading(true);
    try {
      const result = await invoke<CopilotModel[]>("list_models");
      setModels(result);
    } catch {
      // Backend list_models has its own fallback; if it fails too, show empty
      setModels([]);
    } finally {
      setModelsLoading(false);
    }
  }, []);

  const handleLogin = useCallback(async () => {
    invoke("log_action", { action: "Login started" }).catch(() => {});
    setLoginStep("loading");
    setLoginError(null);
    try {
      const codeInfo = await invoke<DeviceCodeInfo>("start_github_login");
      setDeviceCode(codeInfo);
      setLoginStep("code");
    } catch (err) {
      setLoginError(String(err));
      setLoginStep("error");
    }
  }, []);

  const handleCopyAndOpen = useCallback(async () => {
    if (!deviceCode) return;
    // Copy code to clipboard via Rust backend (reliable in Tauri)
    try {
      await invoke("copy_to_clipboard", { text: deviceCode.user_code });
    } catch {
      // Fallback to navigator.clipboard
      try { await navigator.clipboard.writeText(deviceCode.user_code); } catch { /* ignore */ }
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
    // Open browser via Tauri shell plugin (window.open doesn't work in WebView)
    try {
      await open(deviceCode.verification_uri);
    } catch {
      // Fallback: try invoke a Rust command to open URL
      try { await invoke("open_url", { url: deviceCode.verification_uri }); } catch { /* ignore */ }
    }
    setLoginStep("waiting");
    try {
      await invoke("poll_github_login");
      const status = await invoke<AuthStatus>("get_auth_status");
      setAuthStatus(status);
      setLoginStep("idle");
      // Refresh models after login
      fetchModels();
    } catch (err) {
      setLoginError(String(err));
      setLoginStep("error");
    }
  }, [deviceCode]);

  const handleLogout = useCallback(async () => {
    invoke("log_action", { action: "Logout clicked" }).catch(() => {});
    try {
      await invoke("logout");
      setAuthStatus({ logged_in: false, username: null });
    } catch (err) {
      console.error("Logout failed:", err);
    }
  }, []);

  return (
    <div className="min-h-screen bg-gray-50 dark:bg-gray-900 p-5">
      <div className="max-w-md mx-auto">
        <h1 className="text-lg font-bold text-gray-900 dark:text-gray-100 mb-0.5">Copilot Rewrite</h1>
        <p className="text-xs text-gray-500 dark:text-gray-400 mb-4">Settings</p>

        {/* Account Section */}
        <section className="bg-white dark:bg-gray-800 rounded-xl shadow-sm border border-gray-200 dark:border-gray-700 p-5 mb-4">
          <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-3 flex items-center gap-2">
            <svg className="w-4 h-4" viewBox="0 0 16 16" fill="currentColor">
              <path fillRule="evenodd" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
            </svg>
            GitHub Account
          </h2>

          {authStatus.logged_in ? (
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <img
                  src={`https://github.com/${authStatus.username}.png?size=64`}
                  alt={authStatus.username || "User"}
                  className="w-8 h-8 rounded-full"
                  onError={(e) => { (e.target as HTMLImageElement).style.display = "none"; }}
                />
                <div>
                  <p className="text-sm font-medium text-gray-900 dark:text-gray-100">{authStatus.username || "Connected"}</p>
                  <p className="text-xs text-green-600 dark:text-green-400">● Copilot active</p>
                </div>
              </div>
              <button
                onClick={handleLogout}
                className="text-xs text-red-500 hover:text-red-700 transition-colors"
              >
                Sign out
              </button>
            </div>
          ) : loginStep === "idle" ? (
            <button
              onClick={handleLogin}
              className="w-full rounded-lg bg-gray-900 dark:bg-gray-100 px-4 py-2.5 text-sm font-medium text-white dark:text-gray-900 transition-colors hover:bg-gray-800 dark:hover:bg-gray-200 active:scale-[0.98] flex items-center justify-center gap-2"
            >
              <svg className="w-4 h-4" viewBox="0 0 16 16" fill="currentColor">
                <path fillRule="evenodd" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
              </svg>
              Sign in with GitHub
            </button>
          ) : loginStep === "loading" ? (
            <div className="flex items-center justify-center py-4">
              <div className="h-5 w-5 animate-spin rounded-full border-2 border-gray-300 dark:border-gray-600 border-t-gray-900 dark:border-t-gray-100" />
              <span className="ml-3 text-sm text-gray-500 dark:text-gray-400">Connecting...</span>
            </div>
          ) : loginStep === "code" && deviceCode ? (
            <div>
              <p className="text-sm text-gray-600 dark:text-gray-400 mb-2">Copy this code and enter it on GitHub:</p>
              <div className="rounded-lg bg-gray-50 dark:bg-gray-700 border-2 border-dashed border-gray-200 dark:border-gray-600 p-3 mb-3 text-center">
                <span className="font-mono text-xl font-bold tracking-[0.3em] text-gray-900 dark:text-gray-100 select-all">
                  {deviceCode.user_code}
                </span>
              </div>
              <button
                onClick={handleCopyAndOpen}
                className="w-full rounded-lg bg-copilot-blue px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-copilot-blue-hover flex items-center justify-center gap-2"
              >
                {copied ? "✓ Copied!" : "📋 Copy & Open GitHub"}
              </button>
            </div>
          ) : loginStep === "waiting" ? (
            <div className="text-center py-3">
              {deviceCode && (
                <div className="rounded-lg bg-gray-50 dark:bg-gray-700 border border-gray-200 dark:border-gray-600 p-2 mb-3">
                  <span className="font-mono text-lg font-bold tracking-[0.2em] text-gray-400 dark:text-gray-500">{deviceCode.user_code}</span>
                </div>
              )}
              <div className="flex items-center justify-center">
                <div className="h-5 w-5 animate-spin rounded-full border-2 border-copilot-blue border-t-transparent" />
                <span className="ml-3 text-sm text-gray-500 dark:text-gray-400">Waiting for authorization...</span>
              </div>
            </div>
          ) : loginStep === "error" ? (
            <div>
              <div className="rounded-lg bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 p-3 mb-3">
                <p className="text-sm text-red-700 dark:text-red-400">{loginError}</p>
              </div>
              <button
                onClick={handleLogin}
                className="w-full rounded-lg bg-gray-900 dark:bg-gray-100 px-4 py-2.5 text-sm font-medium text-white dark:text-gray-900 hover:bg-gray-800 dark:hover:bg-gray-200"
              >
                Try Again
              </button>
            </div>
          ) : null}
        </section>

        {/* Model Section */}
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
          <div className="flex items-center justify-between mb-1.5">
            <h2 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase">AI Model</h2>
            <button onClick={() => { invoke("log_action", { action: "Refresh models clicked" }).catch(() => {}); fetchModels(); }} className="text-xs text-copilot-blue hover:underline" disabled={modelsLoading}>
              {modelsLoading ? "..." : "↻ Refresh"}
            </button>
          </div>
          <select
            value={settings.model}
            onChange={(e) => {
              invoke("log_action", { action: `Model changed to: ${e.target.value}` }).catch(() => {});
              setSettings({ ...settings, model: e.target.value });
            }}
            className={`w-full rounded border px-2.5 py-1.5 text-sm focus:outline-none focus:ring-1 bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 ${
              !settings.model
                ? "border-red-400 focus:ring-red-500"
                : "border-gray-200 dark:border-gray-600 focus:border-copilot-blue focus:ring-copilot-blue"
            }`}
          >
            {!settings.model && <option value="" disabled>— Select a model —</option>}
            {(() => {
              // Group by vendor, sort groups alphabetically, sort models within each group by name
              const grouped = models.reduce<Record<string, CopilotModel[]>>((acc, m) => {
                const vendor = m.vendor || "Other";
                (acc[vendor] = acc[vendor] || []).push(m);
                return acc;
              }, {});
              const sortedVendors = Object.keys(grouped).sort();
              return sortedVendors.map((vendor) => (
                <optgroup key={vendor} label={vendor}>
                  {grouped[vendor].sort((a, b) => a.name.localeCompare(b.name)).map((model) => (
                    <option key={model.id} value={model.id}>
                      {model.name}{model.preview ? " (Preview)" : ""}{model.category === "powerful" ? " ⚡" : ""}
                    </option>
                  ))}
                </optgroup>
              ));
            })()}
            {models.length === 0 && settings.model && (
              <option value={settings.model}>{settings.model}</option>
            )}
          </select>
          {!settings.model && <p className="text-xs text-red-500 mt-0.5">⚠ Model is required</p>}
          <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-1">Only models that support chat completions are listed.</p>
        </section>

        {/* Language Section */}
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
          <h2 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase mb-1.5">Target Language</h2>
          <select
            value={settings.target_language}
            onChange={(e) => {
              invoke("log_action", { action: `Language changed to: ${e.target.value}` }).catch(() => {});
              setSettings({ ...settings, target_language: e.target.value });
            }}
            className="w-full rounded border border-gray-200 dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 px-2.5 py-1.5 text-sm focus:border-copilot-blue focus:outline-none focus:ring-1 focus:ring-copilot-blue"
          >
            {LANGUAGES.map((lang) => <option key={lang} value={lang}>{lang}</option>)}
          </select>
        </section>

        {/* Native Language Section */}
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
          <h2 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase mb-1.5">Native Language</h2>
          <select
            value={settings.native_language}
            onChange={(e) => {
              invoke("log_action", { action: `Native language changed to: ${e.target.value}` }).catch(() => {});
              setSettings({ ...settings, native_language: e.target.value });
            }}
            className="w-full rounded border border-gray-200 dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 px-2.5 py-1.5 text-sm focus:border-copilot-blue focus:outline-none focus:ring-1 focus:ring-copilot-blue"
          >
            {LANGUAGES.map((lang) => <option key={lang} value={lang}>{lang}</option>)}
          </select>
          <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-1">Your mother tongue. Read Mode translates foreign text to this language.</p>
        </section>

        {/* Read Mode Section */}
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
          <label className="flex items-center justify-between cursor-pointer mb-2">
            <div>
              <span className="text-sm font-medium text-gray-700 dark:text-gray-300">📖 Read Mode</span>
              <p className="text-xs text-gray-400 dark:text-gray-500 mt-0.5">Translate text selected in non-input areas (webpages, PDFs, etc.)</p>
            </div>
            <input
              type="checkbox"
              checked={settings.read_mode_enabled}
              onChange={(e) => {
                invoke("log_action", { action: `Read mode ${e.target.checked ? "enabled" : "disabled"}` }).catch(() => {});
                setSettings({ ...settings, read_mode_enabled: e.target.checked });
              }}
              className="w-4 h-4 rounded border-gray-300 text-copilot-blue focus:ring-copilot-blue ml-3 flex-shrink-0"
            />
          </label>
          {settings.read_mode_enabled && (
            <div className="mt-2 pt-2 border-t border-gray-100 dark:border-gray-700">
              <p className="text-xs text-gray-500 dark:text-gray-400 mb-1.5">Sub-mode</p>
              <div className="flex gap-2">
                <button
                  onClick={() => {
                    invoke("log_action", { action: "Read mode sub changed to: translate_summarize" }).catch(() => {});
                    setSettings({ ...settings, read_mode_sub: "translate_summarize" });
                  }}
                  className={`flex-1 rounded-lg px-3 py-2 text-xs font-medium transition-colors ${
                    settings.read_mode_sub === "translate_summarize"
                      ? "bg-copilot-blue text-white"
                      : "bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-600"
                  }`}
                >
                  Translate + Summarize
                </button>
                <button
                  onClick={() => {
                    invoke("log_action", { action: "Read mode sub changed to: simple_translate" }).catch(() => {});
                    setSettings({ ...settings, read_mode_sub: "simple_translate" });
                  }}
                  className={`flex-1 rounded-lg px-3 py-2 text-xs font-medium transition-colors ${
                    settings.read_mode_sub === "simple_translate"
                      ? "bg-copilot-blue text-white"
                      : "bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-600"
                  }`}
                >
                  Simple Translate
                </button>
              </div>
            </div>
          )}
        </section>

        {/* Beast Mode */}
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
          <label className="flex items-center justify-between cursor-pointer">
            <div>
              <span className="text-sm font-medium text-gray-700 dark:text-gray-300">🐺 Beast Mode</span>
              <p className="text-xs text-gray-400 dark:text-gray-500 mt-0.5">Full creative rewrite — examples, restructuring, best version</p>
            </div>
            <input
              type="checkbox"
              checked={settings.beast_mode}
              onChange={(e) => {
              invoke("log_action", { action: `Beast mode ${e.target.checked ? "enabled" : "disabled"}` }).catch(() => {});
              setSettings({ ...settings, beast_mode: e.target.checked });
            }}
              className="w-4 h-4 rounded border-gray-300 text-copilot-blue focus:ring-copilot-blue ml-3 flex-shrink-0"
            />
          </label>
        </section>

        {/* General */}
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
          <label className="flex items-center justify-between cursor-pointer">
            <span className="text-sm text-gray-700 dark:text-gray-300">Start on Windows login</span>
            <input
              type="checkbox"
              checked={settings.auto_start}
              onChange={(e) => {
              invoke("log_action", { action: `Auto-start ${e.target.checked ? "enabled" : "disabled"}` }).catch(() => {});
              setSettings({ ...settings, auto_start: e.target.checked });
            }}
              className="w-4 h-4 rounded border-gray-300 text-copilot-blue focus:ring-copilot-blue"
            />
          </label>
        </section>

        {/* Theme */}
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
          <h2 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase mb-1.5">Appearance</h2>
          <div className="flex gap-2">
            {(["system", "light", "dark"] as const).map((t) => (
              <button
                key={t}
                onClick={() => {
                  invoke("log_action", { action: `Theme changed to: ${t}` }).catch(() => {});
                  themeCtx.changeTheme(t);
                  setSettings({ ...settings, theme: t });
                }}
                className={`flex-1 rounded-lg px-3 py-2 text-xs font-medium transition-colors ${
                  themeCtx.theme === t
                    ? "bg-copilot-blue text-white"
                    : "bg-gray-100 dark:bg-gray-700 text-gray-600 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-600"
                }`}
              >
                {t === "system" ? "☀️🌙 System" : t === "light" ? "☀️ Light" : "🌙 Dark"}
              </button>
            ))}
          </div>
        </section>

        {saved && <p className="text-center text-xs text-green-500 dark:text-green-400 mt-1">✓ Saved</p>}

        {/* Update Section */}
        {updater.status === "available" && (
          <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-lg px-4 py-3 mt-3">
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm font-medium text-blue-900 dark:text-blue-300">
                  Update available:{" "}
                  <a
                    href="#"
                    onClick={(e) => { e.preventDefault(); open(`https://github.com/wangmingliang-ms/copilot-rewrite/releases/tag/v${updater.version}`); }}
                    className="text-blue-600 hover:underline"
                  >
                    v{updater.version}
                  </a>
                </p>
              </div>
              <button
                onClick={() => {
                  invoke("log_action", { action: `Update Now clicked — downloading v${updater.version}` }).catch(() => {});
                  updater.downloadAndInstall();
                }}
                className="ml-3 rounded-lg bg-blue-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-blue-700 transition-colors whitespace-nowrap"
              >
                Update Now
              </button>
            </div>
          </div>
        )}
        {updater.status === "downloading" && (
          <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-lg px-4 py-3 mt-3">
            <p className="text-sm font-medium text-blue-900 dark:text-blue-300 mb-2">Downloading update... {updater.progress}%</p>
            <div className="w-full bg-blue-200 rounded-full h-1.5">
              <div
                className="bg-blue-600 h-1.5 rounded-full transition-all duration-300"
                style={{ width: `${updater.progress}%` }}
              />
            </div>
          </div>
        )}
        {updater.status === "ready" && (
          <div className="bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg px-4 py-3 mt-3">
            <p className="text-sm font-medium text-green-800 dark:text-green-400">✓ Update installed — restarting...</p>
          </div>
        )}
        {updater.status === "error" && (
          <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg px-4 py-2 mt-3">
            <p className="text-xs text-red-600 dark:text-red-400">{updater.error}</p>
          </div>
        )}

        <div className="flex items-center justify-between mt-3 text-xs text-gray-400 dark:text-gray-500">
          <div className="flex items-center gap-1.5">
            <button
              onClick={() => {
                invoke("log_action", { action: "View Log clicked" }).catch(() => {});
                invoke("open_log_file").catch(() => {});
              }}
              className="hover:text-copilot-blue transition-colors underline"
            >
              View Log
            </button>
            <span>|</span>
            <button
              onClick={() => {
                invoke("log_action", { action: "Log Directory clicked" }).catch(() => {});
                invoke("open_log_dir").catch(() => {});
              }}
              className="hover:text-copilot-blue transition-colors underline"
            >
              Log Directory
            </button>
          </div>
          <div className="flex items-center gap-2">
            {updater.status === "checking" ? (
              <span className="text-gray-400">Checking...</span>
            ) : updater.status === "idle" || updater.status === "upToDate" || updater.status === "error" ? (
              <button
                onClick={() => {
                  invoke("log_action", { action: "Check updates clicked" }).catch(() => {});
                  updater.checkForUpdate();
                }}
                className="hover:text-copilot-blue transition-colors"
                title="Check for updates"
              >
                {updater.status === "upToDate" ? "✓ Up to date" : "Check updates"}
              </button>
            ) : null}
            <a href="#" onClick={(e) => { e.preventDefault(); open(`https://github.com/wangmingliang-ms/copilot-rewrite/releases/tag/v${appVersion}`); }} className="hover:underline">v{appVersion}</a>
          </div>
        </div>
      </div>
    </div>
  );
};

export default SettingsPanel;
