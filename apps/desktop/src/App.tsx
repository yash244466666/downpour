import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AddDownloadDialog } from "./components/AddDownloadDialog";
import { BatchImportDialog } from "./components/BatchImportDialog";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { DownloadList } from "./components/DownloadList";
import { Header } from "./components/Header";
import { SettingsDialog } from "./components/SettingsDialog";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { SummaryCards } from "./components/SummaryCards";
import { Toast } from "./components/Toast";
import { initialDownloads, type DemoDownload } from "./data/mock";
import {
  rowFromEngine,
  type DownloadKind,
  type DownloadRow,
} from "./lib/downloads";
import * as engine from "./lib/engine";
import type { MediaTools, Settings } from "./lib/engine";
import { fileNameFromUrl, guessKind } from "./lib/format";
import { filterDownloads, type Filter } from "./lib/filters";
import { useSpeedHistory } from "./lib/speedHistory";
import { useTheme } from "./lib/theme";

const FALLBACK_SETTINGS: Settings = {
  downloadDir: "~/Downloads",
  connections: 8,
  speedLimit: 0,
  theme: "system",
  maxConcurrent: 3,
  maxPerHost: 4,
  scheduleEnabled: false,
  scheduleStart: "00:00",
  scheduleEnd: "23:59",
  scheduleDays: 0b01111111,
  autostart: false,
};

function mockRow(url: string, connections: number): DemoDownload {
  const name = fileNameFromUrl(url);
  return {
    id: crypto.randomUUID(),
    name,
    host: (() => {
      try {
        return new URL(url).host;
      } catch {
        return "direct link";
      }
    })(),
    kind: guessKind(name),
    size: 0,
    downloaded: 0,
    speed: 0,
    connections,
    status: "queued",
    added: "just now",
  };
}

export default function App() {
  const { theme, toggle } = useTheme();
  const [downloads, setDownloads] = useState<DownloadRow[]>(initialDownloads);
  const [preview, setPreview] = useState(true);
  const [settings, setSettings] = useState<Settings>(FALLBACK_SETTINGS);
  const [mediaTools, setMediaTools] = useState<MediaTools | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [category, setCategory] = useState<DownloadKind | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [batchOpen, setBatchOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<DownloadRow | null>(null);
  const [clearOpen, setClearOpen] = useState(false);
  const [connectionError, setConnectionError] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const engineSeen = useRef(false);

  const refresh = useCallback(async () => {
    try {
      const [records, nextSettings, tools] = await Promise.all([
        engine.listDownloads(),
        engine.getSettings(),
        engine.getMediaTools().catch(() => null),
      ]);
      setDownloads(records.map((record) => rowFromEngine(record)));
      setSettings(nextSettings);
      if (tools) setMediaTools(tools);
      setPreview(false);
      engineSeen.current = true;
      setConnectionError(false);
    } catch {
      // Browser preview: keep the mock rows and the preview badge. In the real
      // app, surface a connection problem instead of silently freezing.
      if (engineSeen.current) setConnectionError(true);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => {
      void refresh();
    }, 1200);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const visible = useMemo(
    () => filterDownloads(downloads, filter, category, query),
    [downloads, filter, category, query],
  );

  const counts = useMemo(
    () => ({
      all: downloads.length,
      active: downloads.filter(
        (item) => item.status === "downloading" || item.status === "paused",
      ).length,
      queued: downloads.filter((item) => item.status === "queued").length,
      completed: downloads.filter((item) => item.status === "completed").length,
    }),
    [downloads],
  );

  const categoryCounts = useMemo(() => {
    const result: Partial<Record<DownloadKind, number>> = {};
    for (const download of downloads) {
      result[download.kind] = (result[download.kind] ?? 0) + 1;
    }
    return result;
  }, [downloads]);

  const clearable = useMemo(
    () =>
      downloads.filter((item) =>
        ["completed", "failed", "canceled"].includes(item.status),
      ).length,
    [downloads],
  );

  const totalSpeed = useMemo(
    () =>
      downloads
        .filter((item) => item.status === "downloading")
        .reduce((total, item) => total + item.speed, 0),
    [downloads],
  );

  const speedHistory = useSpeedHistory(totalSpeed);

  async function addDownload(
    url: string,
    connections: number,
    startNow: boolean,
  ) {
    const pending: DownloadRow = {
      ...mockRow(url, connections),
      id: `pending-${crypto.randomUUID()}`,
    };
    setDialogOpen(false);
    setFilter("all");
    setCategory(null);

    if (preview) {
      setDownloads((list) => [pending, ...list]);
      setToast("Preview mode — start the Tauri app to download.");
      return;
    }

    // Show the row immediately; the refresh replaces it with the real record.
    setDownloads((list) => [pending, ...list]);
    try {
      await engine.addDownload({
        url,
        connections,
        startNow,
        directory: settings.downloadDir,
      });
      setToast(`Added “${fileNameFromUrl(url)}”.`);
      await refresh();
    } catch (error) {
      setDownloads((list) => list.filter((item) => item.id !== pending.id));
      setToast(String(error));
    }
  }

  async function toggleDownload(id: string) {
    const download = downloads.find((item) => item.id === id);
    if (!download) return;

    if (preview) {
      setDownloads((list) =>
        list.map((item) => {
          if (item.id !== id) return item;
          if (item.status === "downloading") {
            return { ...item, status: "paused", speed: 0, eta: undefined };
          }
          if (item.status === "paused" || item.status === "queued") {
            return { ...item, status: "downloading" };
          }
          return item;
        }),
      );
      return;
    }

    try {
      if (download.status === "downloading") {
        await engine.pauseDownload(id);
      } else if (
        download.status === "paused" ||
        download.status === "queued"
      ) {
        await engine.startDownload(id);
      }
      await refresh();
    } catch (error) {
      setToast(String(error));
    }
  }

  async function cancelDownload(id: string) {
    if (preview) {
      setDownloads((list) =>
        list.map((item) =>
          item.id === id
            ? { ...item, status: "canceled", speed: 0, eta: undefined }
            : item,
        ),
      );
      return;
    }
    try {
      await engine.cancelDownload(id);
      await refresh();
    } catch (error) {
      setToast(String(error));
    }
  }

  async function removeDownload(id: string, deleteFile: boolean) {
    setDeleteTarget(null);
    if (preview) {
      setDownloads((list) => list.filter((item) => item.id !== id));
      return;
    }
    try {
      await engine.removeDownload(id, deleteFile);
      await refresh();
    } catch (error) {
      setToast(String(error));
    }
  }

  async function clearFinished() {
    setClearOpen(false);
    if (preview) {
      setDownloads((list) =>
        list.filter(
          (item) => !["completed", "failed", "canceled"].includes(item.status),
        ),
      );
      return;
    }
    try {
      const count = await engine.clearFinished();
      setToast(
        `Removed ${count} finished download${count === 1 ? "" : "s"} from the list.`,
      );
      await refresh();
    } catch (error) {
      setToast(String(error));
    }
  }

  async function revealDownload(download: DownloadRow) {
    const target =
      download.finalPath ?? download.directory ?? settings.downloadDir;
    if (!target) return;
    if (preview) {
      setToast("Preview mode — there is no file on disk yet.");
      return;
    }
    try {
      await engine.revealPath(target);
    } catch (error) {
      setToast(String(error));
    }
  }

  async function addBatch(urls: string[]) {
    setFilter("all");
    setCategory(null);

    if (preview) {
      setDownloads((list) => [
        ...urls.slice(0, 500).map((url) => mockRow(url, settings.connections)),
        ...list,
      ]);
      setBatchOpen(false);
      setToast("Preview mode — start the Tauri app to download.");
      return;
    }

    try {
      const result = await engine.addDownloads({
        urls: urls.slice(0, 500),
        directory: settings.downloadDir,
        connections: settings.connections,
      });
      setBatchOpen(false);
      const firstFailure = result.failed[0];
      setToast(
        result.failed.length > 0
          ? `Queued ${result.added}; ${result.failed.length} failed${firstFailure ? ` — ${firstFailure}` : ""}.`
          : `Queued ${result.added} downloads.`,
      );
      await refresh();
    } catch (error) {
      setToast(String(error));
    }
  }

  async function changePriority(id: string, delta: number) {
    const download = downloads.find((item) => item.id === id);
    if (!download || preview) return;
    try {
      await engine.setPriority(id, (download.priority ?? 0) + delta);
      await refresh();
    } catch (error) {
      setToast(String(error));
    }
  }

  async function startAll() {
    if (preview) {
      setDownloads((list) =>
        list.map((item) =>
          item.status === "queued" || item.status === "paused"
            ? { ...item, status: "downloading" }
            : item,
        ),
      );
      return;
    }
    try {
      await engine.startAll();
      await refresh();
    } catch (error) {
      setToast(String(error));
    }
  }

  async function pauseAll() {
    if (preview) {
      setDownloads((list) =>
        list.map((item) =>
          item.status === "downloading" || item.status === "queued"
            ? { ...item, status: "paused", speed: 0, eta: undefined }
            : item,
        ),
      );
      return;
    }
    try {
      await engine.pauseAll();
      await refresh();
    } catch (error) {
      setToast(String(error));
    }
  }

  async function saveSettings(next: Settings) {
    setSettings(next);
    setSettingsOpen(false);
    if (preview) {
      setToast("Settings saved for this preview session.");
      return;
    }
    try {
      await engine.setSettings(next);
      setToast("Settings saved.");
    } catch (error) {
      setToast(String(error));
    }
  }

  return (
    <div className="flex h-full bg-bg text-fg">
      <Sidebar
        categoryCounts={categoryCounts}
        category={category}
        onCategory={setCategory}
        totalSpeed={totalSpeed}
        speedHistory={speedHistory}
        preview={preview}
        onOpenFolder={() => {
          void (async () => {
            if (preview) {
              setToast("Preview mode — no download folder yet.");
              return;
            }
            try {
              await engine.revealPath(settings.downloadDir);
            } catch (error) {
              setToast(String(error));
            }
          })();
        }}
      />

      <div className="flex min-w-0 flex-1 flex-col">
        <Header
          filter={filter}
          counts={counts}
          totalSpeed={totalSpeed}
          query={query}
          onQuery={setQuery}
          theme={theme}
          onToggleTheme={toggle}
          onAdd={() => setDialogOpen(true)}
          onSettings={() => setSettingsOpen(true)}
        />

        <main className="flex-1 overflow-y-auto">
          <div className="mx-auto w-full max-w-[1440px] px-4 pb-10 pt-5 sm:px-5 lg:px-8 lg:pt-6">
            {connectionError && (
              <div
                role="alert"
                className="mb-4 flex items-center justify-between gap-3 rounded-xl border border-danger/30 bg-danger-soft px-4 py-3 text-[12px] text-danger"
              >
                <span>Cannot reach the engine. Showing the last known list.</span>
                <button
                  type="button"
                  onClick={() => void refresh()}
                  className="rounded-md border border-danger/30 px-2.5 py-1 text-[11px] font-medium"
                >
                  Retry
                </button>
              </div>
            )}
            <SummaryCards downloads={downloads} speedHistory={speedHistory} />
            <DownloadList
              downloads={visible}
              total={downloads.length}
              filter={filter}
              preview={preview}
              onFilter={setFilter}
              onToggle={(id) => void toggleDownload(id)}
              onCancel={(id) => void cancelDownload(id)}
              onDelete={(download) => setDeleteTarget(download)}
              onReveal={(download) => void revealDownload(download)}
              onPriority={(id, delta) => void changePriority(id, delta)}
              onStartAll={() => void startAll()}
              onPauseAll={() => void pauseAll()}
              onBatch={() => setBatchOpen(true)}
              onClear={() => setClearOpen(true)}
              clearableCount={clearable}
              onShowAll={() => {
                setFilter("all");
                setCategory(null);
                setQuery("");
              }}
            />
          </div>
        </main>

        <StatusBar preview={preview} />
      </div>

      {dialogOpen && (
        <AddDownloadDialog
          defaultDir={settings.downloadDir}
          onClose={() => setDialogOpen(false)}
          onAdd={(url, connections, startNow) =>
            void addDownload(url, connections, startNow)
          }
        />
      )}

      {batchOpen && (
        <BatchImportDialog
          onClose={() => setBatchOpen(false)}
          onAdd={(urls) => void addBatch(urls)}
        />
      )}

      {settingsOpen && (
        <SettingsDialog
          settings={settings}
          mediaTools={mediaTools}
          preview={preview}
          onClose={() => setSettingsOpen(false)}
          onSave={(next) => void saveSettings(next)}
        />
      )}

      {clearOpen && (
        <ConfirmDialog
          title="Clear finished downloads?"
          message={`Remove ${clearable} completed, failed, or cancelled item${
            clearable === 1 ? "" : "s"
          } from the list? Files already on disk are kept.`}
          onClose={() => setClearOpen(false)}
          actions={[
            {
              label: `Clear ${clearable}`,
              tone: "danger",
              onClick: () => void clearFinished(),
            },
          ]}
        />
      )}

      {deleteTarget && (
        <ConfirmDialog
          title="Remove download?"
          message={`“${deleteTarget.name}”`}
          onClose={() => setDeleteTarget(null)}
          actions={[
            {
              label: "Remove from list",
              tone: "muted",
              onClick: () => void removeDownload(deleteTarget.id, false),
            },
            ...(deleteTarget.finalPath
              ? [
                  {
                    label: "Delete file",
                    tone: "danger" as const,
                    onClick: () => void removeDownload(deleteTarget.id, true),
                  },
                ]
              : []),
          ]}
        >
          <div className="truncate rounded-lg border border-line bg-surface-2 px-3 py-2 font-mono text-[11px] text-muted">
            {deleteTarget.finalPath ?? "No file on disk"}
          </div>
        </ConfirmDialog>
      )}

      {toast && <Toast message={toast} onDismiss={() => setToast(null)} />}
    </div>
  );
}
