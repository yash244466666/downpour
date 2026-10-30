import { useEffect, useRef, useState } from "react";
import type { FormEvent } from "react";
import {
  FolderOpen,
  Gauge,
  Info,
  Link2,
  X,
} from "lucide-react";
import { cn } from "../lib/utils";

export function AddDownloadDialog({
  onClose,
  onAdd,
}: {
  onClose: () => void;
  onAdd: (url: string, connections: number) => void;
}) {
  const [url, setUrl] = useState("");
  const [connections, setConnections] = useState(8);
  const [startNow, setStartNow] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  function submit(event: FormEvent) {
    event.preventDefault();
    const trimmed = url.trim();
    if (!/^https?:\/\/\S+$/i.test(trimmed)) {
      setError("Enter a valid http:// or https:// URL.");
      return;
    }
    setError(null);
    onAdd(trimmed, connections);
  }

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-black/50 p-4 backdrop-blur-sm"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <form
        role="dialog"
        aria-modal="true"
        aria-labelledby="add-download-title"
        onSubmit={submit}
        className="animate-toast-in w-full max-w-lg overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl"
      >
        <div className="flex items-start gap-3 border-b border-line px-5 py-4">
          <div className="mt-0.5 grid h-9 w-9 place-items-center rounded-xl bg-accent-soft text-accent">
            <Link2 className="h-4 w-4" />
          </div>
          <div>
            <h2 id="add-download-title" className="text-sm font-semibold">
              Add download
            </h2>
            <p className="mt-0.5 text-[11px] text-muted">
              Paste a direct link, a media page, or a manifest URL.
            </p>
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

        <div className="space-y-4 px-5 py-5">
          <label className="block">
            <span className="mb-1.5 block text-[11px] font-medium text-muted">
              Download URL
            </span>
            <input
              ref={inputRef}
              value={url}
              onChange={(event) => {
                setUrl(event.target.value);
                if (error) setError(null);
              }}
              placeholder="https://example.com/file.zip"
              spellCheck={false}
              className={cn(
                "h-10 w-full rounded-lg border bg-surface-2 px-3 text-[13px] outline-none transition-colors placeholder:text-muted/60",
                error
                  ? "border-danger/60 focus:ring-2 focus:ring-danger/15"
                  : "border-line focus:border-accent/50 focus:ring-2 focus:ring-accent/15",
              )}
            />
            {error && (
              <span className="mt-1.5 block text-[11px] text-danger">{error}</span>
            )}
          </label>

          <div className="flex items-center gap-2 rounded-lg border border-line bg-surface-2 px-3 py-2.5">
            <FolderOpen className="h-4 w-4 shrink-0 text-muted" />
            <div className="min-w-0 flex-1">
              <div className="text-[11px] text-muted">Save to</div>
              <div className="truncate text-[12px]">~/Downloads</div>
            </div>
            <button
              type="button"
              disabled
              className="cursor-not-allowed rounded-md border border-line px-2 py-1 text-[10px] text-muted opacity-60"
              title="Destination picker arrives with the engine"
            >
              Change
            </button>
          </div>

          <div className="rounded-lg border border-line bg-surface-2 px-3 py-3">
            <div className="flex items-center justify-between">
              <span className="inline-flex items-center gap-1.5 text-[11px] font-medium text-muted">
                <Gauge className="h-3.5 w-3.5" />
                Parallel connections
              </span>
              <span className="rounded-md bg-accent-soft px-2 py-0.5 text-[11px] font-semibold tabular-nums text-accent">
                {connections}
              </span>
            </div>
            <input
              type="range"
              min={1}
              max={32}
              value={connections}
              onChange={(event) => setConnections(Number(event.target.value))}
              aria-label="Parallel connections"
              className="mt-3 w-full accent-[var(--accent)]"
            />
            <div className="mt-1 flex justify-between text-[10px] text-muted">
              <span>1</span>
              <span>8 recommended</span>
              <span>32</span>
            </div>
          </div>

          <div className="flex items-center justify-between rounded-lg border border-line px-3 py-2.5">
            <div>
              <div className="text-[12px] font-medium">Start immediately</div>
              <div className="text-[10px] text-muted">
                Otherwise it waits in the queue.
              </div>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={startNow}
              aria-label="Start immediately"
              onClick={() => setStartNow((value) => !value)}
              className={cn(
                "relative h-5 w-9 rounded-full transition-colors",
                startNow ? "bg-accent" : "bg-line",
              )}
            >
              <span
                className={cn(
                  "absolute top-0.5 h-4 w-4 rounded-full bg-white shadow-sm transition-[left]",
                  startNow ? "left-[18px]" : "left-0.5",
                )}
              />
            </button>
          </div>

          <div className="flex gap-2 rounded-lg border border-line bg-surface-2 px-3 py-2.5 text-[11px] leading-relaxed text-muted">
            <Info className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>
              This is an M0 UI preview. Submitting adds a local mock entry; the
              real engine, auth, proxy, and checksum options arrive in M1–M2.
            </span>
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-line bg-surface-2/40 px-5 py-3">
          <button
            type="button"
            onClick={onClose}
            className="h-9 rounded-lg border border-line bg-surface px-3 text-[12px] font-medium text-muted transition-colors hover:text-fg"
          >
            Cancel
          </button>
          <button
            type="submit"
            className="h-9 rounded-lg bg-accent px-4 text-[12px] font-semibold text-white shadow-sm transition-[filter] hover:brightness-110"
          >
            Add download
          </button>
        </div>
      </form>
    </div>
  );
}
