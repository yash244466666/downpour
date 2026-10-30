import type { DownloadKind } from "./downloads";

export function formatBytes(bytes: number, decimals = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    units.length - 1,
  );
  const value = bytes / 1024 ** exponent;
  const digits = exponent === 0 ? 0 : decimals;
  return `${value.toFixed(digits)} ${units[exponent]}`;
}

export function formatSpeed(bytesPerSecond: number): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) return "—";
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatEta(seconds?: number): string {
  if (!seconds || !Number.isFinite(seconds) || seconds <= 0) return "—";
  const total = Math.round(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${secs}s`;
  return `${secs}s`;
}

export function percent(downloaded: number, total: number): number {
  if (!total || total <= 0) return 0;
  return Math.min(100, Math.max(0, (downloaded / total) * 100));
}

export function fileNameFromUrl(rawUrl: string): string {
  try {
    const url = new URL(rawUrl);
    const last = url.pathname.split("/").filter(Boolean).pop();
    if (last) return decodeURIComponent(last);
    return url.hostname || "download";
  } catch {
    return "download";
  }
}

export function guessKind(fileName: string): DownloadKind {
  const extension = fileName.split(".").pop()?.toLowerCase() ?? "";
  if (["mp4", "mkv", "webm", "mov", "avi", "m4v"].includes(extension))
    return "video";
  if (["mp3", "m4a", "flac", "wav", "ogg", "opus"].includes(extension))
    return "audio";
  if (["pdf", "doc", "docx", "txt", "md", "epub", "csv", "xlsx"].includes(extension))
    return "document";
  if (["zip", "rar", "7z", "tar", "gz", "xz", "bz2", "zst"].includes(extension))
    return "archive";
  if (["exe", "msi", "dmg", "appimage", "deb", "rpm", "iso", "pkg"].includes(extension))
    return "app";
  if (["png", "jpg", "jpeg", "gif", "webp", "svg", "avif"].includes(extension))
    return "image";
  return "other";
}
