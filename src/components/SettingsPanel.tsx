import { useState, useEffect, useCallback, type FC } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-shell";
import { getVersion } from "@tauri-apps/api/app";
import { useUpdater } from "../hooks/useUpdater";
import * as Select from "@radix-ui/react-select";
import * as Checkbox from "@radix-ui/react-checkbox";
import * as Progress from "@radix-ui/react-progress";
import { RefreshCw, Check, ChevronDown, ArrowUpLeft, ArrowUp, ArrowUpRight, ArrowDownLeft, ArrowDown, ArrowDownRight } from "lucide-react";

// GitHub Octocat logo — brand mark not available in Lucide
const GithubIcon = ({ size = 16, className = "" }: { size?: number; className?: string }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" className={className}>
    <path fillRule="evenodd" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
  </svg>
);

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
  creative_mode: boolean;
  model: string;
  theme: string;
  native_language: string;
  read_mode_enabled: boolean;
  read_mode_sub: string;
  popup_icon_position: string;
  debug_mode: boolean;
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
    creative_mode: false,
    model: "claude-sonnet-4",
    theme: "system",
    native_language: "Chinese (Simplified)",
    read_mode_enabled: true,
    read_mode_sub: "translate_summarize",
    popup_icon_position: "top-left",
    debug_mode: false,
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
        invoke("log_action", { action: `Settings saved — model=${settings.model}, lang=${settings.target_language}, creative=${settings.creative_mode}, autoStart=${settings.auto_start}` }).catch(() => {});
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
            <GithubIcon size={16} />
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
              <GithubIcon size={16} />
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
            <button onClick={() => { invoke("log_action", { action: "Refresh models clicked" }).catch(() => {}); fetchModels(); }} className="flex items-center gap-1 text-xs text-copilot-blue hover:underline" disabled={modelsLoading}>
              {modelsLoading ? "..." : <><RefreshCw size={12} /> Refresh</>}
            </button>
          </div>
          <Select.Root
            value={settings.model}
            onValueChange={(value) => {
              invoke("log_action", { action: `Model changed to: ${value}` }).catch(() => {});
              setSettings({ ...settings, model: value });
            }}
          >
            <Select.Trigger
              className={`w-full rounded border px-2.5 py-1.5 text-sm focus:outline-none focus:ring-1 bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 flex items-center justify-between ${
                !settings.model
                  ? "border-red-400 focus:ring-red-500"
                  : "border-gray-200 dark:border-gray-600 focus:border-copilot-blue focus:ring-copilot-blue"
              }`}
            >
              <Select.Value placeholder="— Select a model —" />
              <Select.Icon>
                <ChevronDown size={14} className="text-gray-400" />
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
                            className="px-2.5 py-1.5 text-sm text-gray-900 dark:text-gray-100 rounded cursor-pointer outline-none data-[highlighted]:bg-gray-100 dark:data-[highlighted]:bg-gray-700 flex items-center gap-2"
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
                  {models.length === 0 && settings.model && (
                    <Select.Item value={settings.model} className="px-2.5 py-1.5 text-sm text-gray-900 dark:text-gray-100">
                      <Select.ItemText>{settings.model}</Select.ItemText>
                    </Select.Item>
                  )}
                </Select.Viewport>
              </Select.Content>
            </Select.Portal>
          </Select.Root>
          {!settings.model && <p className="text-xs text-red-500 mt-0.5">⚠ Model is required</p>}
          <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-1">Only models that support chat completions are listed.</p>
        </section>

        {/* ── Read Assistant ── */}
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
          <label className="flex items-center justify-between cursor-pointer">
            <span className="text-sm font-semibold text-gray-700 dark:text-gray-300">📖 Read Assistant</span>
            <Checkbox.Root
              checked={settings.read_mode_enabled}
              onCheckedChange={(checked) => {
                invoke("log_action", { action: `Read assistant ${checked ? "enabled" : "disabled"}` }).catch(() => {});
                setSettings({ ...settings, read_mode_enabled: !!checked });
              }}
              className={`w-4 h-4 rounded border flex items-center justify-center ml-3 flex-shrink-0 transition-colors ${
                settings.read_mode_enabled
                  ? "bg-copilot-blue border-copilot-blue"
                  : "border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700"
              }`}
            >
              <Checkbox.Indicator>
                <Check size={12} className="text-white" />
              </Checkbox.Indicator>
            </Checkbox.Root>
          </label>
          <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-1 mb-3">Select text on webpages, PDFs, or messages to translate and understand. AI auto-selects the best mode.</p>

          <div className="space-y-2.5">
            <div>
              <label className="text-[11px] font-medium text-gray-500 dark:text-gray-400">Target Language</label>
              <Select.Root
                value={settings.native_language}
                onValueChange={(value) => {
                  invoke("log_action", { action: `Read assistant target language changed to: ${value}` }).catch(() => {});
                  setSettings({ ...settings, native_language: value });
                }}
              >
                <Select.Trigger className="w-full mt-0.5 rounded border border-gray-200 dark:border-gray-600 bg-gray-50 dark:bg-gray-700/50 text-gray-900 dark:text-gray-100 px-2.5 py-1.5 text-sm focus:border-copilot-blue focus:outline-none focus:ring-1 focus:ring-copilot-blue flex items-center justify-between">
                  <Select.Value />
                  <Select.Icon>
                    <ChevronDown size={14} className="text-gray-400" />
                  </Select.Icon>
                </Select.Trigger>
                <Select.Portal>
                  <Select.Content className="bg-white dark:bg-gray-800 rounded-lg shadow-lg border border-gray-200 dark:border-gray-700 overflow-hidden z-50" position="popper" sideOffset={4}>
                    <Select.Viewport className="p-1 max-h-[250px]">
                      {LANGUAGES.map((lang) => (
                        <Select.Item key={lang} value={lang} className="px-2.5 py-1.5 text-sm text-gray-900 dark:text-gray-100 rounded cursor-pointer outline-none data-[highlighted]:bg-gray-100 dark:data-[highlighted]:bg-gray-700 flex items-center gap-2">
                          <Select.ItemText>{lang}</Select.ItemText>
                          <Select.ItemIndicator>
                            <Check size={12} className="text-copilot-blue" />
                          </Select.ItemIndicator>
                        </Select.Item>
                      ))}
                    </Select.Viewport>
                  </Select.Content>
                </Select.Portal>
              </Select.Root>
              <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5">Your mother tongue. Translations, explanations, and vocabulary notes appear in this language.</p>
            </div>
          </div>
        </section>

        {/* ── Write Assistant ── */}
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
          <div className="flex items-center justify-between mb-1">
            <span className="text-sm font-semibold text-gray-700 dark:text-gray-300">✍️ Write Assistant</span>
          </div>
          <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5 mb-3">Select text in input fields to translate, polish, and rewrite. Always on.</p>

          <div className="space-y-2.5">
            <div>
              <label className="text-[11px] font-medium text-gray-500 dark:text-gray-400">Target Language</label>
              <Select.Root
                value={settings.target_language}
                onValueChange={(value) => {
                  invoke("log_action", { action: `Write assistant target language changed to: ${value}` }).catch(() => {});
                  setSettings({ ...settings, target_language: value });
                }}
              >
                <Select.Trigger className="w-full mt-0.5 rounded border border-gray-200 dark:border-gray-600 bg-gray-50 dark:bg-gray-700/50 text-gray-900 dark:text-gray-100 px-2.5 py-1.5 text-sm focus:border-copilot-blue focus:outline-none focus:ring-1 focus:ring-copilot-blue flex items-center justify-between">
                  <Select.Value />
                  <Select.Icon>
                    <ChevronDown size={14} className="text-gray-400" />
                  </Select.Icon>
                </Select.Trigger>
                <Select.Portal>
                  <Select.Content className="bg-white dark:bg-gray-800 rounded-lg shadow-lg border border-gray-200 dark:border-gray-700 overflow-hidden z-50" position="popper" sideOffset={4}>
                    <Select.Viewport className="p-1 max-h-[250px]">
                      {LANGUAGES.map((lang) => (
                        <Select.Item key={lang} value={lang} className="px-2.5 py-1.5 text-sm text-gray-900 dark:text-gray-100 rounded cursor-pointer outline-none data-[highlighted]:bg-gray-100 dark:data-[highlighted]:bg-gray-700 flex items-center gap-2">
                          <Select.ItemText>{lang}</Select.ItemText>
                          <Select.ItemIndicator>
                            <Check size={12} className="text-copilot-blue" />
                          </Select.ItemIndicator>
                        </Select.Item>
                      ))}
                    </Select.Viewport>
                  </Select.Content>
                </Select.Portal>
              </Select.Root>
              <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5">Your translated output language. Final polished text will be in this language.</p>
            </div>
          </div>
        </section>

        {/* ── Popup Icon Position ── */}
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
          <h2 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase mb-2">Popup Icon Position</h2>
          <p className="text-[10px] text-gray-400 dark:text-gray-500 mb-2">Where the icon appears relative to selected text.</p>
          <div className="flex items-center gap-4">
            {/* Visual position picker */}
            <div className="relative w-[120px] h-[80px] flex-shrink-0">
              {/* Selected text representation */}
              <div className="absolute inset-x-3 top-1/2 -translate-y-1/2 h-[20px] rounded bg-blue-100 dark:bg-blue-900/40 border border-blue-200 dark:border-blue-800 flex items-center justify-center">
                <span className="text-[8px] text-blue-400 dark:text-blue-500 font-medium tracking-wider">SELECTED TEXT</span>
              </div>
              {/* Position dots */}
              {([
                { value: "top-left", style: "top-[6px] left-3" },
                { value: "top-center", style: "top-[6px] left-1/2 -translate-x-1/2" },
                { value: "top-right", style: "top-[6px] right-3" },
                { value: "bottom-left", style: "bottom-[6px] left-3" },
                { value: "bottom-center", style: "bottom-[6px] left-1/2 -translate-x-1/2" },
                { value: "bottom-right", style: "bottom-[6px] right-3" },
              ] as const).map((pos) => (
                <button
                  key={pos.value}
                  onClick={() => {
                    invoke("log_action", { action: `Popup icon position changed to: ${pos.value}` }).catch(() => {});
                    setSettings({ ...settings, popup_icon_position: pos.value });
                  }}
                  className={`absolute ${pos.style} w-4 h-4 rounded-full border-2 transition-all duration-200 ${
                    settings.popup_icon_position === pos.value
                      ? "bg-copilot-blue border-copilot-blue scale-110 shadow-md shadow-blue-300/50 dark:shadow-blue-500/30"
                      : "bg-white dark:bg-gray-600 border-gray-300 dark:border-gray-500 hover:border-copilot-blue hover:scale-105"
                  }`}
                  title={pos.value.replace("-", " ")}
                />
              ))}
            </div>
            {/* Current selection label */}
            <div className="flex items-center gap-1.5 text-xs">
              {(() => {
                const iconClass = "w-4 h-4 text-copilot-blue";
                const pos = settings.popup_icon_position;
                if (pos === "top-left") return <ArrowUpLeft className={iconClass} />;
                if (pos === "top-center") return <ArrowUp className={iconClass} />;
                if (pos === "top-right") return <ArrowUpRight className={iconClass} />;
                if (pos === "bottom-left") return <ArrowDownLeft className={iconClass} />;
                if (pos === "bottom-center") return <ArrowDown className={iconClass} />;
                return <ArrowDownRight className={iconClass} />;
              })()}
              <span className="font-medium text-gray-700 dark:text-gray-300">{
                settings.popup_icon_position === "top-left" ? "Top Left" :
                settings.popup_icon_position === "top-center" ? "Top Center" :
                settings.popup_icon_position === "top-right" ? "Top Right" :
                settings.popup_icon_position === "bottom-left" ? "Bottom Left" :
                settings.popup_icon_position === "bottom-center" ? "Bottom Center" :
                "Bottom Right"
              }</span>
            </div>
          </div>
        </section>

        {/* General */}
        <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
          <label className="flex items-center justify-between cursor-pointer">
            <span className="text-sm text-gray-700 dark:text-gray-300">Start on Windows login</span>
            <Checkbox.Root
              checked={settings.auto_start}
              onCheckedChange={(checked) => {
                invoke("log_action", { action: `Auto-start ${checked ? "enabled" : "disabled"}` }).catch(() => {});
                setSettings({ ...settings, auto_start: !!checked });
              }}
              className={`w-4 h-4 rounded border flex items-center justify-center transition-colors ${
                settings.auto_start
                  ? "bg-copilot-blue border-copilot-blue"
                  : "border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700"
              }`}
            >
              <Checkbox.Indicator>
                <Check size={12} className="text-white" />
              </Checkbox.Indicator>
            </Checkbox.Root>
          </label>
          <label className="flex items-center justify-between cursor-pointer mt-2">
            <div>
              <span className="text-sm text-gray-700 dark:text-gray-300">Debug logging</span>
              <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5">Log LLM prompts and responses to the log file for troubleshooting.</p>
            </div>
            <Checkbox.Root
              checked={settings.debug_mode}
              onCheckedChange={(checked) => {
                invoke("log_action", { action: `Debug mode ${checked ? "enabled" : "disabled"}` }).catch(() => {});
                setSettings({ ...settings, debug_mode: !!checked });
              }}
              className={`w-4 h-4 rounded border flex items-center justify-center ml-3 flex-shrink-0 transition-colors ${
                settings.debug_mode
                  ? "bg-copilot-blue border-copilot-blue"
                  : "border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700"
              }`}
            >
              <Checkbox.Indicator>
                <Check size={12} className="text-white" />
              </Checkbox.Indicator>
            </Checkbox.Root>
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
            <Progress.Root className="w-full bg-blue-200 rounded-full h-1.5 overflow-hidden" value={updater.progress}>
              <Progress.Indicator
                className="bg-blue-600 h-1.5 rounded-full transition-all duration-300"
                style={{ width: `${updater.progress}%` }}
              />
            </Progress.Root>
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
