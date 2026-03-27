import { useState, useEffect } from "react";
import Popup from "./Popup";
import SettingsPanel from "./SettingsPanel";
import { useSelection } from "../hooks/useSelection";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface AuthStatus {
  logged_in: boolean;
  username: string | null;
}

function App() {
  const hash = window.location.hash;

  // ── Settings view ──
  if (hash === "#/settings") {
    return <div className="settings-view"><SettingsPanel /></div>;
  }

  // ── Popup view ── (unified toolbar + preview)
  return <div className="popup-view"><PopupView /></div>;
}

function PopupView() {
  const [authStatus, setAuthStatus] = useState<AuthStatus>({ logged_in: false, username: null });
  const { selection } = useSelection();

  useEffect(() => {
    // Check auth on mount
    invoke<AuthStatus>("get_auth_status").then(setAuthStatus).catch(() => {});

    // Re-check auth on every new selection — picks up login that happened in Settings
    const unsub = listen("selection-detected", () => {
      invoke<AuthStatus>("get_auth_status").then(setAuthStatus).catch(() => {});
    });

    return () => { unsub.then((f) => f()); };
  }, []);

  return <Popup selection={selection} authStatus={authStatus} />;
}

export default App;
