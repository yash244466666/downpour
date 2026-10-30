import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import { CalendarClock, Gauge, FolderOpen, ListOrdered, Settings as SettingsIcon, X } from "lucide-react";
import type { Settings } from "../lib/engine";
import { cn } from "../lib/utils";

const DAY_LABELS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

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
  const [maxConcurrent, setMaxConcurrent] = useState(settings.maxConcurrent);
  const [maxPerHost, setMaxPerHost] = useState(settings.maxPerHost);
  const [scheduleEnabled, setScheduleEnabled] = useState(
    settings.scheduleEnabled,
  );
  const [scheduleStart, setScheduleStart] = useState(settings.scheduleStart);
  const [scheduleEnd, setScheduleEnd] = useState(settings.scheduleEnd);
  const [days, setDays] = useState<boolean[]>(() =>
    Array.from({ length: 7 }, (_, index) =>
      Boolean(settings.scheduleDays & (1 << index)),
    ),
  );
  const [autostart, setAutostart] = useState(settings.autostart);
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
    if (!Number.isInteger(maxConcurrent) || maxConcurrent < 1 || maxConcurrent > 16) {
      setError("Maximum concurrent downloads must be between 1 and 16.");
      return;
    }
    if (!Number.isInteger(maxPerHost) || maxPerHost < 1 || maxPerHost > 32) {
      setError("Per-host limit must be between 1 and 32.");
      return;
    }
    if (
      scheduleEnabled &&
      (!/^\d{1,2}:\d{2}$/.test(scheduleStart.trim()) ||
        !/^\d{1,2}:\d{2}$/.test(scheduleEnd.trim()))
    ) {
      setError("Schedule times must use HH:MM.");
      return;
    }
    const daysMask = days.reduce(
      (mask, enabled, index) => (enabled ? mask | (1 << index) : mask),
      0,
    );
    if (scheduleEnabled && daysMask === 0) {
      setError("Choose at least one schedule day.");
      return;
    }
    setError(null);
    onSave({
      ...settings,
      downloadDir: dir,
      connections,
      speedLimit: Math.round(limitKb) * 1024,
      maxConcurrent,
      maxPerHost,
      scheduleEnabled,
      scheduleStart: scheduleStart.trim(),
      scheduleEnd: scheduleEnd.trim(),
      scheduleDays: daysMask,
      autostart,
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
        className="animate-toast-in flex max-h-[90vh] w-full max-w-lg flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl"
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
              Downloads, queue, schedule, and startup.
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

        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 py-5">
          <section className="space-y-4">
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

            <div className="grid grid-cols-2 gap-3">
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
                  className="h-10 w-full rounded-lg border border-line bg-surface-2 px-3 text-[13px] tabular-nums outline-none focus:border-accent/50 focus:ring-2 focus:ring-accent/15"
                />
              </label>
              <label className="block">
                <span className="mb-1.5 inline-flex items-center gap-1.5 text-[11px] font-medium text-muted">
                  <Gauge className="h-3.5 w-3.5" />
                  Speed limit (KB/s)
                </span>
                <input
                  type="number"
                  min={0}
                  value={limitKb}
                  onChange={(event) => setLimitKb(Number(event.target.value))}
                  className="h-10 w-full rounded-lg border border-line bg-surface-2 px-3 text-[13px] tabular-nums outline-none focus:border-accent/50 focus:ring-2 focus:ring-accent/15"
                />
              </label>
            </div>
          </section>

          <section className="rounded-xl border border-line px-3 py-3">
            <div className="mb-3 inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted">
              <ListOrdered className="h-3.5 w-3.5" />
              Queue
            </div>
            <div className="grid grid-cols-2 gap-3">
              <label className="block">
                <span className="mb-1.5 block text-[11px] text-muted">
                  Concurrent downloads
                </span>
                <input
                  type="number"
                  min={1}
                  max={16}
                  value={maxConcurrent}
                  onChange={(event) =>
                    setMaxConcurrent(Number(event.target.value))
                  }
                  className="h-9 w-full rounded-lg border border-line bg-surface-2 px-3 text-[13px] tabular-nums outline-none focus:border-accent/50 focus:ring-2 focus:ring-accent/15"
                />
              </label>
              <label className="block">
                <span className="mb-1.5 block text-[11px] text-muted">
                  Per host
                </span>
                <input
                  type="number"
                  min={1}
                  max={32}
                  value={maxPerHost}
                  onChange={(event) => setMaxPerHost(Number(event.target.value))}
                  className="h-9 w-full rounded-lg border border-line bg-surface-2 px-3 text-[13px] tabular-nums outline-none focus:border-accent/50 focus:ring-2 focus:ring-accent/15"
                />
              </label>
            </div>
            <p className="mt-2 text-[10px] leading-relaxed text-muted">
              Higher-priority queued downloads start first. Use the arrows on a
              queued row to change its priority.
            </p>
          </section>

          <section className="rounded-xl border border-line px-3 py-3">
            <div className="flex items-center justify-between">
              <span className="inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted">
                <CalendarClock className="h-3.5 w-3.5" />
                Schedule
              </span>
              <Toggle
                checked={scheduleEnabled}
                label="Enable schedule"
                onChange={setScheduleEnabled}
              />
            </div>
            <div className="mt-3 grid grid-cols-2 gap-3">
              <label className="block">
                <span className="mb-1.5 block text-[11px] text-muted">
                  Start
                </span>
                <input
                  type="time"
                  value={scheduleStart}
                  onChange={(event) => setScheduleStart(event.target.value)}
                  className="h-9 w-full rounded-lg border border-line bg-surface-2 px-3 text-[13px] tabular-nums outline-none focus:border-accent/50 focus:ring-2 focus:ring-accent/15"
                />
              </label>
              <label className="block">
                <span className="mb-1.5 block text-[11px] text-muted">Stop</span>
                <input
                  type="time"
                  value={scheduleEnd}
                  onChange={(event) => setScheduleEnd(event.target.value)}
                  className="h-9 w-full rounded-lg border border-line bg-surface-2 px-3 text-[13px] tabular-nums outline-none focus:border-accent/50 focus:ring-2 focus:ring-accent/15"
                />
              </label>
            </div>
            <div className="mt-3 flex flex-wrap gap-1.5">
              {DAY_LABELS.map((label, index) => {
                const active = days[index];
                return (
                  <button
                    key={label}
                    type="button"
                    aria-pressed={active}
                    aria-label={label}
                    onClick={() =>
                      setDays((current) =>
                        current.map((value, position) =>
                          position === index ? !value : value,
                        ),
                      )
                    }
                    className={cn(
                      "h-7 rounded-md border px-2 text-[10px] font-medium transition-colors",
                      active
                        ? "border-accent/30 bg-accent-soft text-accent"
                        : "border-line text-muted hover:text-fg",
                    )}
                  >
                    {label}
                  </button>
                );
              })}
            </div>
            <p className="mt-2 text-[10px] leading-relaxed text-muted">
              Outside the window, running downloads pause and queued ones wait.
            </p>
          </section>

          <section className="flex items-center justify-between rounded-xl border border-line px-3 py-2.5">
            <div>
              <div className="text-[12px] font-medium">Start with the system</div>
              <div className="text-[10px] text-muted">
                Opens Downpour when you sign in.
              </div>
            </div>
            <Toggle
              checked={autostart}
              label="Start with the system"
              onChange={setAutostart}
            />
          </section>

          {error && (
            <p className="text-[11px] text-danger" role="alert">
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

function Toggle({
  checked,
  label,
  onChange,
}: {
  checked: boolean;
  label: string;
  onChange: (value: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative h-5 w-9 rounded-full transition-colors",
        checked ? "bg-accent" : "bg-line",
      )}
    >
      <span
        className={cn(
          "absolute top-0.5 h-4 w-4 rounded-full bg-white shadow-sm transition-[left]",
          checked ? "left-[18px]" : "left-0.5",
        )}
      />
    </button>
  );
}
