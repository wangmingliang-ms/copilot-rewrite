import { useState, useCallback, type FC } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-shell";
import * as Dialog from "@radix-ui/react-dialog";

// GitHub Octocat logo — brand mark not available in Lucide
const GithubIcon = ({ size = 24, className = "" }: { size?: number; className?: string }) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" className={className}>
    <path fillRule="evenodd" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" />
  </svg>
);

interface DeviceCodeInfo {
  user_code: string;
  verification_uri: string;
}

interface LoginDialogProps {
  onSuccess: () => void;
  onCancel: () => void;
}

const LoginDialog: FC<LoginDialogProps> = ({ onSuccess, onCancel }) => {
  const [step, setStep] = useState<"prompt" | "loading" | "code" | "waiting" | "success" | "error">("prompt");
  const [deviceCode, setDeviceCode] = useState<DeviceCodeInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const handleLogin = useCallback(async () => {
    setStep("loading");
    setError(null);
    try {
      const codeInfo = await invoke<DeviceCodeInfo>("start_github_login");
      setDeviceCode(codeInfo);
      setStep("code");
    } catch (err) {
      setError(String(err));
      setStep("error");
    }
  }, []);

  const handleCopyAndOpen = useCallback(async () => {
    if (!deviceCode) return;
    
    // Copy code to clipboard via Rust backend (reliable in Tauri)
    try {
      await invoke("copy_to_clipboard", { text: deviceCode.user_code });
    } catch {
      try { await navigator.clipboard.writeText(deviceCode.user_code); } catch { /* ignore */ }
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);

    // Open browser via Tauri shell plugin (window.open doesn't work in WebView)
    try {
      await open(deviceCode.verification_uri);
    } catch {
      try { await invoke("open_url", { url: deviceCode.verification_uri }); } catch { /* ignore */ }
    }

    // Start polling
    setStep("waiting");
    try {
      await invoke("poll_github_login");
      setStep("success");
      setTimeout(() => onSuccess(), 1500);
    } catch (err) {
      setError(String(err));
      setStep("error");
    }
  }, [deviceCode, onSuccess]);

  const handleCopyOnly = useCallback(async () => {
    if (!deviceCode) return;
    try {
      await invoke("copy_to_clipboard", { text: deviceCode.user_code });
    } catch {
      try { await navigator.clipboard.writeText(deviceCode.user_code); } catch { /* ignore */ }
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [deviceCode]);

  return (
    <Dialog.Root open={true} onOpenChange={(open) => { if (!open) onCancel(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 bg-black/30 z-50 data-[state=open]:animate-fade-in" />
        <Dialog.Content className="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 bg-white rounded-xl shadow-2xl p-6 max-w-sm w-full mx-4 animate-fade-in z-50 focus:outline-none">
          {/* Header */}
          <div className="flex items-center gap-3 mb-4">
            <div className="flex items-center justify-center w-10 h-10 rounded-full bg-gray-100">
              <GithubIcon size={24} className="text-gray-800" />
            </div>
            <div>
              <Dialog.Title className="text-base font-semibold text-gray-900">Sign in to GitHub</Dialog.Title>
              <Dialog.Description className="text-xs text-gray-500">Required for Copilot translation</Dialog.Description>
            </div>
          </div>

        {/* Content */}
        {step === "prompt" && (
          <>
            <p className="text-sm text-gray-600 mb-4">
              Copilot Rewrite uses GitHub Copilot to translate and polish your text. 
              You need a GitHub account with Copilot access.
            </p>
            <div className="flex gap-2">
              <button
                onClick={handleLogin}
                className="flex-1 rounded-lg bg-gray-900 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-gray-800 active:scale-[0.98]"
              >
                Sign in with GitHub
              </button>
              <button
                onClick={onCancel}
                className="rounded-lg border border-gray-200 px-4 py-2.5 text-sm text-gray-600 transition-colors hover:bg-gray-50"
              >
                Cancel
              </button>
            </div>
          </>
        )}

        {step === "loading" && (
          <div className="flex items-center justify-center py-6">
            <div className="h-6 w-6 animate-spin rounded-full border-2 border-gray-300 border-t-gray-900" />
            <span className="ml-3 text-sm text-gray-500">Connecting to GitHub...</span>
          </div>
        )}

        {step === "code" && deviceCode && (
          <>
            <p className="text-sm text-gray-600 mb-3">
              Copy this code and enter it on GitHub:
            </p>
            <div 
              className="flex items-center justify-center rounded-lg bg-gray-50 border-2 border-dashed border-gray-200 p-4 mb-3 cursor-pointer hover:bg-gray-100 transition-colors"
              onClick={handleCopyOnly}
              title="Click to copy"
            >
              <span className="font-mono text-2xl font-bold tracking-[0.3em] text-gray-900 select-all">
                {deviceCode.user_code}
              </span>
            </div>
            <div className="flex gap-2">
              <button
                onClick={handleCopyAndOpen}
                className="flex-1 rounded-lg bg-copilot-blue px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-copilot-blue-hover active:scale-[0.98] flex items-center justify-center gap-2"
              >
                {copied ? "✓ Copied!" : "📋 Copy & Open GitHub"}
              </button>
              <button
                onClick={onCancel}
                className="rounded-lg border border-gray-200 px-3 py-2.5 text-sm text-gray-500 transition-colors hover:bg-gray-50"
              >
                ✕
              </button>
            </div>
          </>
        )}

        {step === "waiting" && (
          <>
            <p className="text-sm text-gray-600 mb-3">
              Enter the code on GitHub and authorize access:
            </p>
            {deviceCode && (
              <div className="flex items-center justify-center rounded-lg bg-gray-50 border border-gray-200 p-3 mb-3">
                <span className="font-mono text-lg font-bold tracking-[0.2em] text-gray-400">
                  {deviceCode.user_code}
                </span>
              </div>
            )}
            <div className="flex items-center justify-center py-2">
              <div className="h-5 w-5 animate-spin rounded-full border-2 border-copilot-blue border-t-transparent" />
              <span className="ml-3 text-sm text-gray-500">Waiting for authorization...</span>
            </div>
          </>
        )}

        {step === "success" && (
          <div className="flex flex-col items-center py-4">
            <div className="flex items-center justify-center w-12 h-12 rounded-full bg-green-100 mb-3">
              <span className="text-2xl">✅</span>
            </div>
            <p className="text-sm font-medium text-green-700">Successfully logged in!</p>
          </div>
        )}

        {step === "error" && (
          <>
            <div className="rounded-lg bg-red-50 border border-red-200 p-3 mb-4">
              <p className="text-sm text-red-700">{error}</p>
            </div>
            <div className="flex gap-2">
              <button
                onClick={handleLogin}
                className="flex-1 rounded-lg bg-gray-900 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-gray-800"
              >
                Try Again
              </button>
              <button
                onClick={onCancel}
                className="rounded-lg border border-gray-200 px-4 py-2.5 text-sm text-gray-600 transition-colors hover:bg-gray-50"
              >
                Cancel
              </button>
            </div>
          </>
        )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
};

export default LoginDialog;
