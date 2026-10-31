import { guessKind } from "./format";

export type DownloadKind =
  | "video"
  | "audio"
  | "document"
  | "archive"
  | "app"
  | "image"
  | "other";

export type DownloadStatus =
  | "downloading"
  | "paused"
  | "queued"
  | "completed"
  | "failed"
  | "canceled";

export interface DownloadRow {
  id: string;
  url: string;
  name: string;
  host: string;
  kind: DownloadKind;
  size: number;
  downloaded: number;
  speed: number;
  connections: number;
  status: DownloadStatus;
  eta?: number;
  added: string;
  directory?: string;
  finalPath?: string;
  error?: string;
  createdAtMs?: number;
  priority?: number;
}

/** Wire shape returned by the Tauri `list_downloads` command. */
export interface EngineDownload {
  id: string;
  url: string;
  name: string;
  directory: string;
  state: string;
  totalBytes: number | null;
  downloadedBytes: number;
  speedBps: number;
  connections: number;
  etaSeconds: number | null;
  error: string | null;
  finalPath: string | null;
  createdAtMs: number;
  completedAtMs: number | null;
  priority: number;
}

export function statusFromState(state: string): DownloadStatus {
  switch (state) {
    case "downloading":
    case "probing":
    case "finalizing":
      return "downloading";
    case "paused":
      return "paused";
    case "queued":
      return "queued";
    case "completed":
      return "completed";
    case "canceled":
      return "canceled";
    default:
      return "failed";
  }
}

export function hostFromUrl(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return "direct link";
  }
}

export function formatRelativeTime(ms: number, now = Date.now()): string {
  if (!Number.isFinite(ms) || ms <= 0) return "just now";
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} hr ago`;
  const days = Math.floor(hours / 24);
  if (days === 1) return "yesterday";
  if (days < 7) return `${days} days ago`;
  return new Date(ms).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

export function rowFromEngine(
  record: EngineDownload,
  now = Date.now(),
): DownloadRow {
  const status = statusFromState(record.state);
  const createdMs =
    status === "completed" && record.completedAtMs
      ? record.completedAtMs
      : record.createdAtMs;

  return {
    id: record.id,
    url: record.url,
    name: record.name,
    host: hostFromUrl(record.url),
    kind: guessKind(record.name),
    size: record.totalBytes ?? 0,
    downloaded: record.downloadedBytes,
    speed: record.speedBps,
    connections: record.connections,
    status,
    eta: record.etaSeconds ?? undefined,
    added: formatRelativeTime(createdMs, now),
    directory: record.directory,
    finalPath: record.finalPath ?? undefined,
    error: record.error ?? undefined,
    createdAtMs: record.createdAtMs,
    priority: record.priority,
  };
}
