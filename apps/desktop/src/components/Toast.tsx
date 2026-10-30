import { useEffect } from "react";
import { CircleCheck } from "lucide-react";

export function Toast({
  message,
  onDismiss,
}: {
  message: string;
  onDismiss: () => void;
}) {
  useEffect(() => {
    const timer = setTimeout(onDismiss, 4200);
    return () => clearTimeout(timer);
  }, [message, onDismiss]);

  return (
    <div
      role="status"
      className="animate-toast-in fixed bottom-12 right-4 z-50 flex max-w-sm items-start gap-2.5 rounded-xl border border-line bg-surface px-4 py-3 shadow-xl sm:right-6"
    >
      <CircleCheck className="mt-0.5 h-4 w-4 shrink-0 text-success" />
      <p className="text-[12px] leading-relaxed text-fg">{message}</p>
      <button
        type="button"
        onClick={onDismiss}
        className="ml-1 text-[11px] text-muted hover:text-fg"
        aria-label="Dismiss notification"
      >
        Dismiss
      </button>
    </div>
  );
}
