import { useCallback, useEffect, useMemo, useState } from "react";
import { AddDownloadDialog } from "./components/AddDownloadDialog";
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
import type { Settings } from "./lib/engine";
import { fileNameFromUrl, guessKind } from "./lib/format";
import type { Filter } from "./lib/filters";
import { useTheme } from "./lib/theme";

const FALLBACK_SETTINGS: Settings = {
  downloadDir: "~/Downloads",
  connections: 8,
  speedLimit: 0,
  theme: "system",
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
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [toast, setToast] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [records, nextSettings] = await Promise.all([
        engine.listDownloads(),
        engine.getSettings(),
      ]);
      setDownloads(records.map((record) => rowFromEngine(record)));
      setSettings(nextSettings);
      setPreview(false);
    } catch {
      // Browser preview: keep the mock rows and the preview badge.
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => {
      void refresh();
    }, 1200);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return downloads.filter((download) => {
      const matchesQuery =
        !needle ||
        download.name.toLowerCase().includes(needle) ||
        download.host.toLowerCase().includes(needle);
      const matchesFilter =
        filter === "all" ||
        (filter === "active" &&
          (download.status === "downloading" ||
            download.status === "paused")) ||
        (filter === "queued" && download.status === "queued") ||
        (filter === "completed" && download.status === "completed");
      return matchesQuery && matchesFilter;
    });
  }, [downloads, filter, query]);

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

  const totalSpeed = useMemo(
    () =>
      downloads
        .filter((item) => item.status === "downloading")
        .reduce((total, item) => total + item.speed, 0),
    [downloads],
  );

  async function addDownload(
    url: string,
    connections: number,
    startNow: boolean,
  ) {
    try {
      await engine.addDownload({
        url,
        connections,
        startNow,
        directory: settings.downloadDir,
      });
      setDialogOpen(false);
      setToast(`Added “${fileNameFromUrl(url)}”.`);
      await refresh();
    } catch (error) {
      if (preview) {
        setDownloads((list) => [mockRow(url, connections), ...list]);
        setDialogOpen(false);
        setToast("Preview mode — start the Tauri app to download.");
        return;
      }
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

  async function removeDownload(id: string) {
    if (preview) {
      setDownloads((list) => list.filter((item) => item.id !== id));
      return;
    }
    try {
      await engine.removeDownload(id);
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
        counts={counts}
        categoryCounts={categoryCounts}
        filter={filter}
        onFilter={setFilter}
        totalSpeed={totalSpeed}
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
            <SummaryCards downloads={downloads} />
            <DownloadList
              downloads={visible}
              total={downloads.length}
              filter={filter}
              preview={preview}
              onFilter={setFilter}
              onToggle={(id) => void toggleDownload(id)}
              onCancel={(id) => void cancelDownload(id)}
              onRemove={(id) => void removeDownload(id)}
              onReveal={(download) => void revealDownload(download)}
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

      {settingsOpen && (
        <SettingsDialog
          settings={settings}
          onClose={() => setSettingsOpen(false)}
          onSave={(next) => void saveSettings(next)}
        />
      )}

      {toast && <Toast message={toast} onDismiss={() => setToast(null)} />}
    </div>
  );
}
