import { useMemo, useState } from "react";
import { AddDownloadDialog } from "./components/AddDownloadDialog";
import { DownloadList } from "./components/DownloadList";
import { Header } from "./components/Header";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { SummaryCards } from "./components/SummaryCards";
import { Toast } from "./components/Toast";
import {
  initialDownloads,
  type DemoDownload,
} from "./data/mock";
import { fileNameFromUrl, guessKind } from "./lib/format";
import type { Filter } from "./lib/filters";
import { useTheme } from "./lib/theme";

export default function App() {
  const { theme, toggle } = useTheme();
  const [downloads, setDownloads] =
    useState<DemoDownload[]>(initialDownloads);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [toast, setToast] = useState<string | null>(null);

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
      active: downloads.filter((item) => item.status === "downloading").length,
      queued: downloads.filter((item) => item.status === "queued").length,
      completed: downloads.filter((item) => item.status === "completed").length,
    }),
    [downloads],
  );

  const totalSpeed = useMemo(
    () =>
      downloads
        .filter((item) => item.status === "downloading")
        .reduce((total, item) => total + item.speed, 0),
    [downloads],
  );

  function toggleDownload(id: string) {
    setDownloads((list) =>
      list.map((download) => {
        if (download.id !== id) return download;
        if (download.status === "downloading") {
          return { ...download, status: "paused", speed: 0, eta: undefined };
        }
        if (download.status === "paused" || download.status === "queued") {
          return {
            ...download,
            status: "downloading",
            speed: 12_500_000,
            eta: 120,
          };
        }
        return download;
      }),
    );
  }

  function addDownload(url: string, connections: number) {
    let host = "direct link";
    try {
      host = new URL(url).host;
    } catch {
      // The dialog already validated the URL.
    }
    const name = fileNameFromUrl(url);
    const entry: DemoDownload = {
      id: crypto.randomUUID(),
      name,
      host,
      kind: guessKind(name),
      size: 0,
      downloaded: 0,
      speed: 0,
      connections,
      status: "queued",
      added: "just now",
    };
    setDownloads((list) => [entry, ...list]);
    setDialogOpen(false);
    setToast(`Queued “${name}” — preview only until the engine lands.`);
  }

  return (
    <div className="flex h-full bg-bg text-fg">
      <Sidebar
        counts={counts}
        filter={filter}
        onFilter={setFilter}
        totalSpeed={totalSpeed}
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
        />

        <main className="flex-1 overflow-y-auto">
          <div className="mx-auto w-full max-w-[1440px] px-4 pb-10 pt-5 sm:px-5 lg:px-8 lg:pt-6">
            <SummaryCards downloads={downloads} />
            <DownloadList
              downloads={visible}
              total={downloads.length}
              filter={filter}
              onFilter={setFilter}
              onToggle={toggleDownload}
            />
          </div>
        </main>

        <StatusBar />
      </div>

      {dialogOpen && (
        <AddDownloadDialog
          onClose={() => setDialogOpen(false)}
          onAdd={addDownload}
        />
      )}

      {toast && (
        <Toast message={toast} onDismiss={() => setToast(null)} />
      )}
    </div>
  );
}
