import { useState, useEffect, useCallback, type FC } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-shell";

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
  model: string;
}

const LANGUAGES = [
  "English", "Chinese (Simplified)", "Chinese (Traditional)",
  "Japanese", "Korean", "French", "German", "Spanish",
  "Portuguese", "Russian", "Arabic", "Hindi", "Italian",
];

const MODELS = [
  { id: "gpt-4o", name: "GPT-4o", desc: "Fast & capable" },
  { id: "gpt-4o-mini", name: "GPT-4o Mini", desc: "Fastest, lightweight" },
  { id: "gpt-4.1", name: "GPT-4.1", desc: "Latest GPT-4" },
  { id: "gpt-4.1-mini", name: "GPT-4.1 Mini", desc: "Fast GPT-4.1" },
  { id: "o3-mini", name: "o3 Mini", desc: "Reasoning model" },
  { id: "o4-mini", name: "o4 Mini", desc: "Latest reasoning" },
  { id: "claude-3.5-sonnet", name: "Claude 3.5 Sonnet", desc: "Anthropic, balanced" },
  { id: "claude-3.7-sonnet", name: "Claude 3.7 Sonnet", desc: "Anthropic, latest" },
  { id: "gemini-2.0-flash", name: "Gemini 2.0 Flash", desc: "Google, fast" },
];

interface CopilotModel {
  id: string;
  name: string;
  version: string;
  owned_by: string;
}

const SettingsPanel: FC = () => {
  const [authStatus, setAuthStatus] = useState<AuthStatus>({ logged_in: false, username: null });
  const [settings, setSettings] = useState<Settings>({
    target_language: "English",
    auto_start: false,
    poll_interval_ms: 100,
    model: "claude-sonnet-4",
  });
  const [loginStep, setLoginStep] = useState<"idle" | "loading" | "code" | "waiting" | "error">("idle");
  const [deviceCode, setDeviceCode] = useState<DeviceCodeInfo | null>(null);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [saved, setSaved] = useState(false);
  const [models, setModels] = useState<CopilotModel[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);

  // Load on mount
  const [initialLoaded, setInitialLoaded] = useState(false);
  useEffect(() => {
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
      // Use fallback static list
      setModels(MODELS.map(m => ({ id: m.id, name: m.name, version: "", owned_by: m.desc })));
    } finally {
      setModelsLoading(false);
    }
  }, []);

  const handleLogin = useCallback(async () => {
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
    try {
      await invoke("logout");
      setAuthStatus({ logged_in: false, username: null });
    } catch (err) {
      console.error("Logout failed:", err);
    }
  }, []);

  return (
    <div className="min-h-screen bg-gray-50 p-6">
      <div className="max-w-md mx-auto">
        <h1 className="text-xl font-bold text-gray-900 mb-1">Copilot Rewrite</h1>
        <p className="text-sm text-gray-500 mb-6">Settings</p>

        {/* Account Section */}
        <section className="bg-white rounded-xl shadow-sm border border-gray-200 p-5 mb-4">
          <h2 className="text-sm font-semibold text-gray-700 mb-3 flex items-center gap-2">
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
                  <p className="text-sm font-medium text-gray-900">{authStatus.username || "Connected"}</p>
                  <p className="text-xs text-green-600">● Copilot active</p>
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
              className="w-full rounded-lg bg-gray-900 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-gray-800 active:scale-[0.98] flex items-center justify-center gap-2"
            >
              <svg className="w-4 h-4" viewBox="0 0 16 16" fill="currentColor">
                <path fillRule="evenodd" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
              </svg>
              Sign in with GitHub
            </button>
          ) : loginStep === "loading" ? (
            <div className="flex items-center justify-center py-4">
              <div className="h-5 w-5 animate-spin rounded-full border-2 border-gray-300 border-t-gray-900" />
              <span className="ml-3 text-sm text-gray-500">Connecting...</span>
            </div>
          ) : loginStep === "code" && deviceCode ? (
            <div>
              <p className="text-sm text-gray-600 mb-2">Copy this code and enter it on GitHub:</p>
              <div className="rounded-lg bg-gray-50 border-2 border-dashed border-gray-200 p-3 mb-3 text-center">
                <span className="font-mono text-xl font-bold tracking-[0.3em] text-gray-900 select-all">
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
                <div className="rounded-lg bg-gray-50 border border-gray-200 p-2 mb-3">
                  <span className="font-mono text-lg font-bold tracking-[0.2em] text-gray-400">{deviceCode.user_code}</span>
                </div>
              )}
              <div className="flex items-center justify-center">
                <div className="h-5 w-5 animate-spin rounded-full border-2 border-copilot-blue border-t-transparent" />
                <span className="ml-3 text-sm text-gray-500">Waiting for authorization...</span>
              </div>
            </div>
          ) : loginStep === "error" ? (
            <div>
              <div className="rounded-lg bg-red-50 border border-red-200 p-3 mb-3">
                <p className="text-sm text-red-700">{loginError}</p>
              </div>
              <button
                onClick={handleLogin}
                className="w-full rounded-lg bg-gray-900 px-4 py-2.5 text-sm font-medium text-white hover:bg-gray-800"
              >
                Try Again
              </button>
            </div>
          ) : null}
        </section>

        {/* Language Section */}
        <section className="bg-white rounded-xl shadow-sm border border-gray-200 p-5 mb-4">
          <h2 className="text-sm font-semibold text-gray-700 mb-3">🌐 Target Language</h2>
          <select
            value={settings.target_language}
            onChange={(e) => setSettings({ ...settings, target_language: e.target.value })}
            className="w-full rounded-lg border border-gray-200 px-3 py-2 text-sm text-gray-900 focus:border-copilot-blue focus:outline-none focus:ring-1 focus:ring-copilot-blue"
          >
            {LANGUAGES.map((lang) => (
              <option key={lang} value={lang}>{lang}</option>
            ))}
          </select>
          <p className="text-xs text-gray-400 mt-2">Source language is auto-detected</p>
        </section>

        {/* Model Section */}
        <section className="bg-white rounded-xl shadow-sm border border-gray-200 p-5 mb-4">
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-sm font-semibold text-gray-700">🤖 AI Model</h2>
            <button
              onClick={fetchModels}
              className="text-xs text-copilot-blue hover:underline"
              disabled={modelsLoading}
            >
              {modelsLoading ? "Loading..." : "↻ Refresh"}
            </button>
          </div>
          <select
            value={settings.model}
            onChange={(e) => setSettings({ ...settings, model: e.target.value })}
            className={`w-full rounded-lg border px-3 py-2.5 text-sm text-gray-900 focus:outline-none focus:ring-1 ${
              !settings.model
                ? "border-red-400 focus:border-red-500 focus:ring-red-500"
                : "border-gray-200 focus:border-copilot-blue focus:ring-copilot-blue"
            }`}
          >
            {!settings.model && (
              <option value="" disabled>— Select a model (required) —</option>
            )}
            {models.map((model) => (
              <option key={model.id} value={model.id}>
                {model.name} — {model.owned_by || model.id}
              </option>
            ))}
            {models.length === 0 && settings.model && (
              <option value={settings.model}>{settings.model}</option>
            )}
          </select>
          {!settings.model && (
            <p className="text-xs text-red-500 mt-1">⚠ Model is required. Please select one.</p>
          )}
        </section>

        {/* General Section */}
        <section className="bg-white rounded-xl shadow-sm border border-gray-200 p-5 mb-4">
          <h2 className="text-sm font-semibold text-gray-700 mb-3">⚙️ General</h2>
          <label className="flex items-center justify-between cursor-pointer">
            <span className="text-sm text-gray-700">Start on Windows login</span>
            <input
              type="checkbox"
              checked={settings.auto_start}
              onChange={(e) => setSettings({ ...settings, auto_start: e.target.checked })}
              className="w-4 h-4 rounded border-gray-300 text-copilot-blue focus:ring-copilot-blue"
            />
          </label>
        </section>

        {/* Auto-save indicator */}
        {saved && (
          <p className="text-center text-xs text-green-500 mt-2 transition-opacity">✓ Saved</p>
        )}

        <p className="text-center text-xs text-gray-400 mt-4">Copilot Rewrite v0.1.0</p>
      </div>
    </div>
  );
};

export default SettingsPanel;
