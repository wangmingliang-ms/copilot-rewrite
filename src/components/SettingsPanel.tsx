import { useState, useEffect, useCallback, useRef, type FC } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-shell";
import { getVersion } from "@tauri-apps/api/app";
import { useUpdater } from "../hooks/useUpdater";
import * as Select from "@radix-ui/react-select";
import * as Checkbox from "@radix-ui/react-checkbox";
import * as Progress from "@radix-ui/react-progress";
import { Check, ChevronDown, ArrowUpLeft, ArrowUp, ArrowUpRight, ArrowDownLeft, ArrowDown, ArrowDownRight, Plus, Trash2 } from "lucide-react";

const MicrosoftIcon = ({ size = 16 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 20 20" aria-hidden="true">
    <path fill="#f25022" d="M1 1h8.5v8.5H1z" />
    <path fill="#7fba00" d="M10.5 1H19v8.5h-8.5z" />
    <path fill="#00a4ef" d="M1 10.5h8.5V19H1z" />
    <path fill="#ffb900" d="M10.5 10.5H19V19h-8.5z" />
  </svg>
);

type Theme = "system" | "light" | "dark";

interface ThemeCtx {
  theme: Theme;
  resolved: "light" | "dark";
  changeTheme: (t: Theme) => Promise<void>;
}

type ReplaceMode = "Markdown" | "Rendered" | "Plain";

interface ReplaceRule {
  process: string;
  title_contains: string;
  mode: ReplaceMode;
}

interface Settings {
  target_language: string;
  auto_start: boolean;
  poll_interval_ms: number;
  creative_mode: boolean;
  global_replace_mode: ReplaceMode;
  replace_rules: ReplaceRule[];
  theme: string;
  native_language: string;
  read_mode_enabled: boolean;
  read_mode_sub: string;
  popup_icon_position: string;
  debug_mode: boolean;
}

interface AuthStatus {
  logged_in: boolean;
  username: string | null;
  display_name: string | null;
  environment_override: boolean;
}

interface AuthorizationRequest {
  authorization_url: string;
}

const LANGUAGES = [
  "English", "Chinese (Simplified)", "Chinese (Traditional)",
  "Japanese", "Korean", "French", "German", "Spanish",
  "Portuguese", "Russian", "Arabic", "Hindi", "Italian",
];

// ── Tab definitions ──
type SettingsTab = "general" | "assistant" | "replace" | "popup";

const TABS: { id: SettingsTab; label: string; icon: string }[] = [
  { id: "general", label: "General", icon: "\u2699\uFE0F" },
  { id: "assistant", label: "Assistant", icon: "\uD83D\uDCAC" },
  { id: "replace", label: "Replace", icon: "\uD83D\uDD04" },
  { id: "popup", label: "Popup", icon: "\uD83D\uDCCC" },
];

const SettingsPanel: FC<{ themeCtx: ThemeCtx }> = ({ themeCtx }) => {
  const [activeTab, setActiveTab] = useState<SettingsTab>("general");
  const [authStatus, setAuthStatus] = useState<AuthStatus>({
    logged_in: false,
    username: null,
    display_name: null,
    environment_override: false,
  });
  const [settings, setSettings] = useState<Settings>({
    target_language: "English",
    auto_start: false,
    poll_interval_ms: 100,
    creative_mode: false,
    global_replace_mode: "Rendered",
    replace_rules: [],
    theme: "system",
    native_language: "Chinese (Simplified)",
    read_mode_enabled: true,
    read_mode_sub: "translate_summarize",
    popup_icon_position: "top-left",
    debug_mode: false,
  });
  const [loginStep, setLoginStep] = useState<"idle" | "loading" | "waiting" | "error">("idle");
  const [loginError, setLoginError] = useState<string | null>(null);
  const loginPollingRef = useRef(false);
  const loginFlowRef = useRef(0);
  const [saved, setSaved] = useState(false);
  const [appVersion, setAppVersion] = useState("0.0.0");
  const updater = useUpdater(5_000); // auto-check 5s after settings opens

  // Load on mount + reload when window becomes visible (Settings window is
  // hidden on close and re-shown, so mount only fires once)
  const [initialLoaded, setInitialLoaded] = useState(false);
  const loadSettings = useCallback(() => {
    invoke<Settings>("get_settings").then((s) => {
      setSettings(s);
      if (!initialLoaded) setInitialLoaded(true);
    }).catch(() => { if (!initialLoaded) setInitialLoaded(true); });
  }, [initialLoaded]);
  const loadAuthStatus = useCallback(() => {
    invoke<AuthStatus>("get_auth_status")
      .then((status) => {
        setAuthStatus(status);
        if (status.logged_in) setLoginStep("idle");
      })
      .catch((error) => console.error("Failed to load Microsoft auth status:", error));
  }, []);
  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
    loadSettings();
    loadAuthStatus();
  }, []);
  useEffect(() => {
    const onFocus = () => {
      loadSettings();
      loadAuthStatus();
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [loadSettings, loadAuthStatus]);

  // Auto-save settings on any change (skip initial load)
  useEffect(() => {
    if (!initialLoaded) return;
    const timer = setTimeout(() => {
      invoke("update_settings", { settings }).then(() => {
        invoke("log_action", { action: `Settings saved — backend=foundry-agent, lang=${settings.target_language}, creative=${settings.creative_mode}, autoStart=${settings.auto_start}` }).catch(() => {});
        setSaved(true);
        setTimeout(() => setSaved(false), 1500);
      }).catch((err) => console.error("Auto-save failed:", err));
    }, 300);
    return () => clearTimeout(timer);
  }, [settings, initialLoaded]);

  const handleLogin = useCallback(async () => {
    const flowId = ++loginFlowRef.current;
    invoke("log_action", { action: "Microsoft login started" }).catch(() => {});
    setLoginStep("loading");
    setLoginError(null);
    try {
      const request = await invoke<AuthorizationRequest>("start_microsoft_login");
      if (loginFlowRef.current !== flowId) return;
      loginPollingRef.current = true;
      setLoginStep("waiting");
      try {
        await open(request.authorization_url);
      } catch {
        await invoke("open_url", { url: request.authorization_url });
      }

      const status = await invoke<AuthStatus>("poll_microsoft_login");
      if (loginFlowRef.current !== flowId) return;
      setAuthStatus(status);
      setLoginStep("idle");
    } catch (error) {
      if (loginFlowRef.current !== flowId) return;
      setLoginError(String(error));
      setLoginStep("error");
    } finally {
      if (loginFlowRef.current === flowId) {
        loginPollingRef.current = false;
      }
    }
  }, []);

  const handleCancelLogin = useCallback(async () => {
    loginFlowRef.current += 1;
    loginPollingRef.current = false;
    await invoke("cancel_microsoft_login").catch(() => {});
    setLoginError(null);
    setLoginStep("idle");
  }, []);

  const handleLogout = useCallback(async () => {
    loginFlowRef.current += 1;
    invoke("log_action", { action: "Microsoft logout clicked" }).catch(() => {});
    try {
      await invoke("logout");
      setAuthStatus({
        logged_in: false,
        username: null,
        display_name: null,
        environment_override: false,
      });
      setLoginStep("idle");
    } catch (error) {
      setLoginError(String(error));
      setLoginStep("error");
    }
  }, []);

  // ── Tab content render functions ──

  const renderGeneralTab = () => (
    <>
      {/* Microsoft Account */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
        <h2 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase mb-2 flex items-center gap-2">
          <MicrosoftIcon size={14} />
          Microsoft Account
        </h2>

        {authStatus.logged_in ? (
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 min-w-0">
              <div className="w-8 h-8 rounded-full bg-copilot-blue/10 text-copilot-blue flex items-center justify-center flex-shrink-0">
                <MicrosoftIcon size={16} />
              </div>
              <div className="min-w-0">
                <p className="text-sm font-medium text-gray-900 dark:text-gray-100 truncate">
                  {authStatus.display_name || authStatus.username || "Microsoft user"}
                </p>
                <p className="text-[11px] text-gray-500 dark:text-gray-400 truncate">
                  {authStatus.username || "Connected to Microsoft Foundry"}
                </p>
                <p className="text-[10px] text-green-600 dark:text-green-400">{"\u25CF"} Foundry access connected</p>
              </div>
            </div>
            {!authStatus.environment_override && (
              <button
                onClick={handleLogout}
                className="ml-3 text-xs text-red-500 hover:text-red-700 transition-colors flex-shrink-0"
              >
                Sign out
              </button>
            )}
          </div>
        ) : loginStep === "idle" ? (
          <div>
            <button
              onClick={handleLogin}
              className="w-full rounded-lg bg-gray-900 dark:bg-gray-100 px-4 py-2.5 text-sm font-medium text-white dark:text-gray-900 transition-colors hover:bg-gray-800 dark:hover:bg-gray-200 active:scale-[0.98] flex items-center justify-center gap-2"
            >
              <MicrosoftIcon size={16} />
              Sign in with Microsoft
            </button>
            <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-2 text-center">
              Access is limited to users assigned by the administrator.
            </p>
          </div>
        ) : loginStep === "loading" ? (
          <div className="flex items-center justify-center py-4">
            <div className="h-5 w-5 animate-spin rounded-full border-2 border-gray-300 dark:border-gray-600 border-t-gray-900 dark:border-t-gray-100" />
            <span className="ml-3 text-sm text-gray-500 dark:text-gray-400">Connecting...</span>
          </div>
        ) : loginStep === "waiting" ? (
          <div className="text-center py-3">
            <div className="flex items-center justify-center">
              <div className="h-5 w-5 animate-spin rounded-full border-2 border-copilot-blue border-t-transparent" />
              <span className="ml-3 text-sm text-gray-500 dark:text-gray-400">Complete sign-in in your browser...</span>
            </div>
            <button
              onClick={handleCancelLogin}
              className="mt-3 text-xs text-gray-500 dark:text-gray-400 hover:text-gray-700 dark:hover:text-gray-200"
            >
              Cancel
            </button>
          </div>
        ) : loginStep === "error" ? (
          <div>
            <div className="rounded-lg bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 p-3 mb-3">
              <p className="text-sm text-red-700 dark:text-red-400 break-words">{loginError}</p>
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

      {/* Theme / Appearance */}
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
              {t === "system" ? "\u2600\uFE0F\uD83C\uDF19 System" : t === "light" ? "\u2600\uFE0F Light" : "\uD83C\uDF19 Dark"}
            </button>
          ))}
        </div>
      </section>

      {/* General — auto-start + debug */}
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
    </>
  );

  const renderAssistantTab = () => (
    <>
      {/* Read Assistant */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
        <label className="flex items-center justify-between cursor-pointer">
          <span className="text-sm font-semibold text-gray-700 dark:text-gray-300">{"\uD83D\uDCD6"} Read Assistant</span>
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

      {/* Write Assistant */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
        <div className="flex items-center justify-between mb-1">
          <span className="text-sm font-semibold text-gray-700 dark:text-gray-300">{"\u270D\uFE0F"} Write Assistant</span>
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
    </>
  );

  const renderReplaceTab = () => (
    <>
      {/* Smart Replace — Default Mode */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
        <div className="flex items-center justify-between mb-1">
          <span className="text-sm font-semibold text-gray-700 dark:text-gray-300">{"\uD83D\uDD04"} Smart Replace</span>
        </div>
        <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5 mb-3">
          Auto-detect how to paste replaced text based on the target app.
        </p>

        <div className="mb-3">
          <label className="text-[11px] font-medium text-gray-500 dark:text-gray-400">Default Mode</label>
          <Select.Root
            value={settings.global_replace_mode}
            onValueChange={(value) => {
              invoke("log_action", { action: `Global replace mode changed to: ${value}` }).catch(() => {});
              setSettings({ ...settings, global_replace_mode: value as ReplaceMode });
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
                <Select.Viewport className="p-1">
                  {([
                    { value: "Rendered", label: "Rich Text \u2014 HTML paste (Teams, Outlook, Word)" },
                    { value: "Markdown", label: "Markdown \u2014 raw source (GitHub, GitLab, Slack)" },
                    { value: "Plain", label: "Plain Text \u2014 no formatting (Notepad, terminal)" },
                  ] as const).map((opt) => (
                    <Select.Item key={opt.value} value={opt.value} className="px-2.5 py-1.5 text-sm text-gray-900 dark:text-gray-100 rounded cursor-pointer outline-none data-[highlighted]:bg-gray-100 dark:data-[highlighted]:bg-gray-700 flex items-center gap-2">
                      <Select.ItemText>{opt.label}</Select.ItemText>
                      <Select.ItemIndicator>
                        <Check size={12} className="text-copilot-blue" />
                      </Select.ItemIndicator>
                    </Select.Item>
                  ))}
                </Select.Viewport>
              </Select.Content>
            </Select.Portal>
          </Select.Root>
          <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5">Used when no app-specific rule matches.</p>
        </div>
      </section>

      {/* App-Specific Rules */}
      <section className="bg-white dark:bg-gray-800 rounded-lg shadow-sm border border-gray-200 dark:border-gray-700 px-4 py-3 mb-3">
        <h2 className="text-xs font-semibold text-gray-500 dark:text-gray-400 uppercase mb-1.5">App-Specific Rules</h2>
        <p className="text-[10px] text-gray-400 dark:text-gray-500 mb-3">
          Rules are matched top-to-bottom; first match wins. When you change the replace mode in the popup, a rule is automatically created here.
        </p>

        {settings.replace_rules.length > 0 ? (
          <div className="space-y-1.5 mb-3">
            {/* Header row */}
            <div className="grid grid-cols-[1fr_1fr_100px_28px] gap-1.5 items-center">
              <span className="text-[10px] font-medium text-gray-400 dark:text-gray-500 uppercase tracking-wide px-1">Process</span>
              <span className="text-[10px] font-medium text-gray-400 dark:text-gray-500 uppercase tracking-wide px-1">Title Contains</span>
              <span className="text-[10px] font-medium text-gray-400 dark:text-gray-500 uppercase tracking-wide px-1">Mode</span>
              <span />
            </div>
            {settings.replace_rules.map((rule, idx) => (
              <div key={idx} className="grid grid-cols-[1fr_1fr_100px_28px] gap-1.5 items-center">
                <input
                  type="text"
                  value={rule.process}
                  onChange={(e) => {
                    const rules = [...settings.replace_rules];
                    rules[idx] = { ...rules[idx], process: e.target.value };
                    setSettings({ ...settings, replace_rules: rules });
                  }}
                  className="rounded border border-gray-200 dark:border-gray-600 bg-gray-50 dark:bg-gray-700/50 text-gray-900 dark:text-gray-100 px-2 py-1 text-xs focus:border-copilot-blue focus:outline-none focus:ring-1 focus:ring-copilot-blue"
                  placeholder="e.g. chrome"
                />
                <input
                  type="text"
                  value={rule.title_contains}
                  onChange={(e) => {
                    const rules = [...settings.replace_rules];
                    rules[idx] = { ...rules[idx], title_contains: e.target.value };
                    setSettings({ ...settings, replace_rules: rules });
                  }}
                  className="rounded border border-gray-200 dark:border-gray-600 bg-gray-50 dark:bg-gray-700/50 text-gray-900 dark:text-gray-100 px-2 py-1 text-xs focus:border-copilot-blue focus:outline-none focus:ring-1 focus:ring-copilot-blue"
                  placeholder="(optional)"
                />
                <Select.Root
                  value={rule.mode}
                  onValueChange={(value) => {
                    const rules = [...settings.replace_rules];
                    rules[idx] = { ...rules[idx], mode: value as ReplaceMode };
                    setSettings({ ...settings, replace_rules: rules });
                  }}
                >
                  <Select.Trigger className="rounded border border-gray-200 dark:border-gray-600 bg-gray-50 dark:bg-gray-700/50 text-gray-900 dark:text-gray-100 px-2 py-1 text-xs focus:border-copilot-blue focus:outline-none focus:ring-1 focus:ring-copilot-blue flex items-center justify-between">
                    <Select.Value>
                      {rule.mode === "Rendered" ? "Rich" : rule.mode === "Markdown" ? "MD" : "Plain"}
                    </Select.Value>
                    <Select.Icon>
                      <ChevronDown size={10} className="text-gray-400" />
                    </Select.Icon>
                  </Select.Trigger>
                  <Select.Portal>
                    <Select.Content className="bg-white dark:bg-gray-800 rounded-lg shadow-lg border border-gray-200 dark:border-gray-700 overflow-hidden z-50" position="popper" sideOffset={4}>
                      <Select.Viewport className="p-1">
                        <Select.Item value="Rendered" className="px-2.5 py-1 text-xs text-gray-900 dark:text-gray-100 rounded cursor-pointer outline-none data-[highlighted]:bg-gray-100 dark:data-[highlighted]:bg-gray-700 flex items-center gap-2">
                          <Select.ItemText>Rich Text</Select.ItemText>
                          <Select.ItemIndicator><Check size={10} className="text-copilot-blue" /></Select.ItemIndicator>
                        </Select.Item>
                        <Select.Item value="Markdown" className="px-2.5 py-1 text-xs text-gray-900 dark:text-gray-100 rounded cursor-pointer outline-none data-[highlighted]:bg-gray-100 dark:data-[highlighted]:bg-gray-700 flex items-center gap-2">
                          <Select.ItemText>Markdown</Select.ItemText>
                          <Select.ItemIndicator><Check size={10} className="text-copilot-blue" /></Select.ItemIndicator>
                        </Select.Item>
                        <Select.Item value="Plain" className="px-2.5 py-1 text-xs text-gray-900 dark:text-gray-100 rounded cursor-pointer outline-none data-[highlighted]:bg-gray-100 dark:data-[highlighted]:bg-gray-700 flex items-center gap-2">
                          <Select.ItemText>Plain Text</Select.ItemText>
                          <Select.ItemIndicator><Check size={10} className="text-copilot-blue" /></Select.ItemIndicator>
                        </Select.Item>
                      </Select.Viewport>
                    </Select.Content>
                  </Select.Portal>
                </Select.Root>
                <button
                  onClick={() => {
                    const rules = settings.replace_rules.filter((_, i) => i !== idx);
                    setSettings({ ...settings, replace_rules: rules });
                  }}
                  className="flex items-center justify-center w-7 h-7 rounded text-gray-400 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 transition-colors"
                  title="Delete rule"
                >
                  <Trash2 size={14} />
                </button>
              </div>
            ))}
          </div>
        ) : (
          <div className="text-xs text-gray-400 dark:text-gray-500 py-3 mb-3 text-center border border-dashed border-gray-200 dark:border-gray-600 rounded-lg">
            No rules — all apps will use the default mode above.
          </div>
        )}

        <button
          onClick={() => {
            setSettings({
              ...settings,
              replace_rules: [...settings.replace_rules, { process: "", title_contains: "", mode: "Rendered" }],
            });
          }}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium text-copilot-blue border border-copilot-blue/30 hover:bg-copilot-blue/5 dark:hover:bg-copilot-blue/10 transition-colors"
        >
          <Plus size={14} />
          Add Rule
        </button>
      </section>
    </>
  );

  const renderPopupTab = () => (
    <>
      {/* Popup Icon Position */}
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
    </>
  );

  // ── Main render ──
  return (
    <div className="h-screen bg-gray-50 dark:bg-gray-900 flex flex-col overflow-hidden">
      {/* Header */}
      <div className="px-5 pt-4 pb-2 flex-shrink-0">
        <h1 className="text-lg font-bold text-gray-900 dark:text-gray-100 mb-0.5">Copilot Rewrite</h1>
        <p className="text-xs text-gray-500 dark:text-gray-400">Settings</p>
      </div>

      {/* Two-column body */}
      <div className="flex flex-1 min-h-0 px-4">
        {/* Left sidebar — Tab menu */}
        <nav className="w-36 flex-shrink-0 pr-3 py-2 space-y-0.5">
          {TABS.map((tab) => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`w-full text-left px-3 py-2 rounded-md text-sm transition-colors flex items-center gap-2 ${
                activeTab === tab.id
                  ? "bg-copilot-blue/10 text-copilot-blue font-medium border-l-2 border-copilot-blue"
                  : "text-gray-600 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800 border-l-2 border-transparent"
              }`}
            >
              <span className="text-base leading-none">{tab.icon}</span>
              {tab.label}
            </button>
          ))}
        </nav>

        {/* Right content area */}
        <main className="flex-1 overflow-y-auto py-2 pl-3 border-l border-gray-200 dark:border-gray-700">
          {activeTab === "general" && renderGeneralTab()}
          {activeTab === "assistant" && renderAssistantTab()}
          {activeTab === "replace" && renderReplaceTab()}
          {activeTab === "popup" && renderPopupTab()}

          {/* Saved indicator */}
          {saved && <p className="text-center text-xs text-green-500 dark:text-green-400 mt-1">{"\u2713"} Saved</p>}
        </main>
      </div>

      {/* Footer */}
      <div className="flex-shrink-0 border-t border-gray-200 dark:border-gray-700 px-5 py-2">
        {/* Update notifications */}
        {updater.status === "available" && (
          <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-lg px-4 py-3 mb-2">
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
          <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-lg px-4 py-3 mb-2">
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
          <div className="bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg px-4 py-3 mb-2">
            <p className="text-sm font-medium text-green-800 dark:text-green-400">{"\u2713"} Update installed — restarting...</p>
          </div>
        )}
        {updater.status === "error" && (
          <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg px-4 py-2 mb-2">
            <p className="text-xs text-red-600 dark:text-red-400">{updater.error}</p>
          </div>
        )}

        <div className="flex items-center justify-between text-xs text-gray-400 dark:text-gray-500">
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
                {updater.status === "upToDate" ? "\u2713 Up to date" : "Check updates"}
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
