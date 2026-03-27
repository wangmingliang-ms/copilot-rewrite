import Popup from "./Popup";
import SettingsPanel from "./SettingsPanel";
import { useSelection } from "../hooks/useSelection";

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
  const { selection } = useSelection();
  return <Popup selection={selection} />;
}

export default App;
