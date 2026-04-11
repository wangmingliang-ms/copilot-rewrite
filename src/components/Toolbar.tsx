import { useEffect, useState, type FC } from "react";
import { XCircle, Languages } from "lucide-react";
import { SelectionInfo } from "../hooks/useSelection";

interface ToolbarProps {
  selection: SelectionInfo | null;
  loading: boolean;
  error: string | null;
  onAction: () => void;
  onDismiss: () => void;
}

const Toolbar: FC<ToolbarProps> = ({ selection, loading, error, onAction, onDismiss }) => {
  const [clicked, setClicked] = useState(false);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDismiss();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onDismiss]);

  if (!selection && !loading && !error) return null;

  const handleClick = () => {
    console.log("[Toolbar] Button clicked! selection:", selection?.text?.substring(0, 30));
    setClicked(true);
    onAction();
  };

  // Use onPointerDown as backup — more reliable in non-focus windows
  const handlePointerDown = (e: React.PointerEvent) => {
    console.log("[Toolbar] PointerDown event fired");
    e.preventDefault();
    handleClick();
  };

  return (
    <div
      className="w-screen h-screen flex items-center justify-center overflow-hidden"
      style={{ background: clicked ? '#e8f5e9' : 'rgba(255,255,255,0.95)', borderRadius: '24px', border: '1px solid #e0e0e0', boxShadow: '0 4px 12px rgba(0,0,0,0.15)' }}
    >
      {loading ? (
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-blue-500 border-t-transparent" />
      ) : error ? (
        <button
          onPointerDown={() => onDismiss()}
          className="w-full h-full flex items-center justify-center"
          title={error}
          style={{ color: '#ef4444' }}
        >
          <XCircle size={20} />
        </button>
      ) : (
        <button
          onClick={handleClick}
          onPointerDown={handlePointerDown}
          className="w-full h-full flex items-center justify-center"
          style={{ cursor: 'pointer', borderRadius: '24px' }}
          title="Translate & Polish"
        >
          <Languages size={28} className="text-[#0078D4]" />
        </button>
      )}
    </div>
  );
};

export default Toolbar;
