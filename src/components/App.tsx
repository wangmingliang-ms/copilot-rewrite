import Popup from "./Popup";
import SettingsPanel from "./SettingsPanel";
import { useSelection } from "../hooks/useSelection";
import { useTheme } from "../hooks/useTheme";

function App() {
  const hash = window.location.hash;
  const themeCtx = useTheme();

  // ── Settings view ──
  if (hash === "#/settings") {
    return <div className="settings-view"><SettingsPanel themeCtx={themeCtx} /></div>;
  }

  // ── Popup view ── (unified toolbar + preview)
  return <div className="popup-view"><PopupView /></div>;
}

function PopupView() {
  useTheme(); // Apply dark class for popup window too
  const { selection } = useSelection();
  return <Popup selection={selection} />;
}

export default App;
