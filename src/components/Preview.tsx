import { useEffect, type FC } from "react";

export interface ProcessResponse {
  original: string;
  result: string;
  action: string;
}

interface PreviewProps {
  result: ProcessResponse | null;
  loading: boolean;
  error: string | null;
  onReplace: () => void;
  onCopy: () => void;
  onCancel: () => void;
}

const Preview: FC<PreviewProps> = ({
  result,
  loading,
  error,
  onReplace,
  onCopy,
  onCancel,
}) => {
  // Dismiss on Escape
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onCancel();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onCancel]);

  if (loading) {
    return (
      <div className="animate-fade-in flex h-full flex-col items-center justify-center rounded-xl bg-surface p-6 shadow-popup">
        <div className="h-8 w-8 animate-spin rounded-full border-3 border-copilot-blue border-t-transparent" />
        <p className="mt-3 text-sm text-gray-500">Processing with Copilot...</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="animate-fade-in flex h-full flex-col rounded-xl bg-surface p-6 shadow-popup">
        <div className="flex-1">
          <h3 className="text-sm font-semibold text-red-500">Error</h3>
          <p className="mt-2 text-xs text-gray-600">{error}</p>
        </div>
        <div className="mt-4 flex justify-end">
          <button
            onClick={onCancel}
            className="rounded-md bg-gray-100 px-4 py-2 text-xs font-medium text-gray-700 transition-colors hover:bg-gray-200"
          >
            Close
          </button>
        </div>
      </div>
    );
  }

  if (!result) {
    return null;
  }

  return (
    <div className="animate-slide-up flex flex-col rounded-xl bg-surface shadow-popup">
      {/* Result text */}
      <div className="px-4 py-3">
        <div className="text-sm leading-relaxed text-gray-800 whitespace-pre-wrap">
          {result.result}
        </div>
      </div>

      {/* Actions — compact inline */}
      <div className="flex items-center justify-end gap-1.5 border-t border-gray-100 px-3 py-2">
        <button
          onClick={onCancel}
          className="rounded-md px-3 py-1.5 text-xs font-medium text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600"
        >
          ✕
        </button>
        <button
          onClick={onCopy}
          className="rounded-md px-3 py-1.5 text-xs font-medium text-gray-600 transition-colors hover:bg-gray-100"
        >
          📋
        </button>
        <button
          onClick={onReplace}
          className="rounded-md bg-copilot-blue px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-copilot-blue-hover"
        >
          Replace
        </button>
      </div>
    </div>
  );
};

export default Preview;
