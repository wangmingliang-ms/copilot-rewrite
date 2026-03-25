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

const actionLabel = (action: string): string => {
  switch (action) {
    case "Translate":
      return "🌐 Translation";
    case "Polish":
      return "✨ Polished";
    case "TranslateAndPolish":
      return "🔄 Translated & Polished";
    default:
      return "Result";
  }
};

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
    <div className="animate-slide-up flex h-full flex-col rounded-xl bg-surface shadow-popup">
      {/* Header */}
      <div className="border-b border-gray-100 px-4 py-3">
        <h3 className="text-sm font-semibold text-gray-800">
          {actionLabel(result.action)}
        </h3>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-auto px-4 py-3">
        {/* Original text */}
        <div className="mb-3">
          <label className="mb-1 block text-[10px] font-medium uppercase tracking-wider text-gray-400">
            Original
          </label>
          <div className="rounded-lg bg-surface-elevated p-3 text-xs leading-relaxed text-gray-600">
            {result.original}
          </div>
        </div>

        {/* Result text */}
        <div>
          <label className="mb-1 block text-[10px] font-medium uppercase tracking-wider text-copilot-blue">
            Result
          </label>
          <div className="rounded-lg border border-copilot-blue/20 bg-blue-50/30 p-3 text-xs leading-relaxed text-gray-800">
            {result.result}
          </div>
        </div>
      </div>

      {/* Actions */}
      <div className="flex items-center justify-end gap-2 border-t border-gray-100 px-4 py-3">
        <button
          onClick={onCancel}
          className="rounded-md px-4 py-2 text-xs font-medium text-gray-500 transition-colors hover:bg-gray-100"
        >
          Cancel
        </button>
        <button
          onClick={onCopy}
          className="rounded-md bg-gray-100 px-4 py-2 text-xs font-medium text-gray-700 transition-colors hover:bg-gray-200"
        >
          📋 Copy
        </button>
        <button
          onClick={onReplace}
          className="rounded-md bg-copilot-blue px-4 py-2 text-xs font-medium text-white transition-colors hover:bg-copilot-blue-hover"
        >
          ✅ Replace
        </button>
      </div>
    </div>
  );
};

export default Preview;
