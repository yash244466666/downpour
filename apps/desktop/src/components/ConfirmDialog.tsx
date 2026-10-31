import { useEffect } from "react";
import type { ReactNode } from "react";
import { X } from "lucide-react";
import { cn } from "../lib/utils";

export interface ConfirmAction {
  label: string;
  onClick: () => void;
  tone?: "primary" | "danger" | "muted";
}

export function ConfirmDialog({
  title,
  message,
  children,
  actions,
  onClose,
}: {
  title: string;
  message: string;
  children?: ReactNode;
  actions: ConfirmAction[];
  onClose: () => void;
}) {
  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-black/50 p-4 backdrop-blur-sm"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        className="animate-toast-in w-full max-w-md overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl"
      >
        <div className="flex items-start gap-3 border-b border-line px-5 py-4">
          <div>
            <h2 id="confirm-title" className="text-sm font-semibold">
              {title}
            </h2>
            <p className="mt-1 text-[12px] leading-relaxed text-muted">{message}</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close dialog"
            className="ml-auto grid h-8 w-8 place-items-center rounded-lg text-muted transition-colors hover:bg-surface-2 hover:text-fg"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        {children && <div className="px-5 py-4">{children}</div>}
        <div className="flex flex-wrap items-center justify-end gap-2 border-t border-line bg-surface-2/40 px-5 py-3">
          <button
            type="button"
            onClick={onClose}
            className="h-9 rounded-lg border border-line bg-surface px-3 text-[12px] font-medium text-muted transition-colors hover:text-fg"
          >
            Cancel
          </button>
          {actions.map((action) => (
            <button
              key={action.label}
              type="button"
              onClick={action.onClick}
              className={cn(
                "h-9 rounded-lg px-4 text-[12px] font-semibold shadow-sm transition-[filter] hover:brightness-110",
                action.tone === "danger"
                  ? "bg-danger text-white"
                  : action.tone === "primary"
                    ? "bg-accent text-white"
                    : "border border-line bg-surface text-fg",
              )}
            >
              {action.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
