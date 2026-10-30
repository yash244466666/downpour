import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import { Gauge, FolderOpen, Settings as SettingsIcon, X } from "lucide-react";
import type { Settings } from "../lib/engine";
import { cn } from "../lib/utils";

export function SettingsDialog({
  settings,
  onClose,
  onSave,
}: {
  settings: Settings;
  onClose: () => void;
  onSave: (settings: Settings) => void;
}) {
  const [downloadDir, setDownloadDir] = useState(settings.downloadDir);
  const [connections, setConnections] = useState(settings.connections);
  const [limitKb, setLimitKb] = useState(
    Math.round(settings.speedLimit / 1024),
  );
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  function submit(event: FormEvent) {
    event.preventDefault();
    const dir = downloadDir.trim();
    if (!dir) {
      setError("Choose a download folder.");
      return;
    }
    if (!Number.isInteger(connections) || connections < 1 || connections > 32) {
      setError("Connections must be a whole number between 1 and 32.");
      return;
    }
    if (!Number.isFinite(limitKb) || limitKb < 0) {
      setError("Speed limit must be zero or a positive number.");
      return;
    }
    setError(null);
    onSave({
      ...settings,
      downloadDir: dir,
      connections,
      speedLimit: Math.round(limitKb) * 1024,
    });
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
        aria-labelledby="settings-title"
        onSubmit={submit}
        className="animate-toast-in w-full max-w-md overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl"
      >
        <div className="flex items-start gap-3 border-b border-line px-5 py-4">
          <div className="mt-0.5 grid h-9 w-9 place-items-center rounded-xl bg-accent-soft text-accent">
            <SettingsIcon className="h-4 w-4" />
          </div>
          <div>
            <h2 id="settings-title" className="text-sm font-semibold">
              Settings
            </h2>
            <p className="mt-0.5 text-[11px] text-muted">
              Downloads, network, and defaults.
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close settings"
            className="ml-auto grid h-8 w-8 place-items-center rounded-lg text-muted transition-colors hover:bg-surface-2 hover:text-fg"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="space-y-4 px-5 py-5">
          <label className="block">
            <span className="mb-1.5 inline-flex items-center gap-1.5 text-[11px] font-medium text-muted">
              <FolderOpen className="h-3.5 w-3.5" />
              Download folder
            </span>
            <input
              value={downloadDir}
              onChange={(event) => setDownloadDir(event.target.value)}
              spellCheck={false}
              className="h-10 w-full rounded-lg border border-line bg-surface-2 px-3 text-[13px] outline-none transition-colors focus:border-accent/50 focus:ring-2 focus:ring-accent/15"
            />
          </label>

          <label className="block">
            <span className="mb-1.5 block text-[11px] font-medium text-muted">
              Default connections
            </span>
            <input
              type="number"
              min={1}
              max={32}
              value={connections}
              onChange={(event) => setConnections(Number(event.target.value))}
              className="h-10 w-full rounded-lg border border-line bg-surface-2 px-3 text-[13px] tabular-nums outline-none transition-colors focus:border-accent/50 focus:ring-2 focus:ring-accent/15"
            />
          </label>

          <label className="block">
            <span className="mb-1.5 inline-flex items-center gap-1.5 text-[11px] font-medium text-muted">
              <Gauge className="h-3.5 w-3.5" />
              Speed limit (KB/s, 0 = unlimited)
            </span>
            <input
              type="number"
              min={0}
              value={limitKb}
              onChange={(event) => setLimitKb(Number(event.target.value))}
              className="h-10 w-full rounded-lg border border-line bg-surface-2 px-3 text-[13px] tabular-nums outline-none transition-colors focus:border-accent/50 focus:ring-2 focus:ring-accent/15"
            />
          </label>

          {error && (
            <p className={cn("text-[11px] text-danger")} role="alert">
              {error}
            </p>
          )}
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
            Save settings
          </button>
        </div>
      </form>
    </div>
  );
}
