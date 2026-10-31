import { invoke } from "@tauri-apps/api/core";
import type { EngineDownload } from "./downloads";

export interface EngineInfo {
  version: string;
  protocol_version: number;
  os: string;
  arch: string;
}

export interface MediaTools {
  ytDlp: string | null;
  ffmpeg: string | null;
}

export interface BatchResult {
  added: number;
  failed: string[];
}

export interface UpdateStatus {
  configured: boolean;
  version: string | null;
  message: string;
}

export interface Settings {
  downloadDir: string;
  connections: number;
  speedLimit: number;
  theme: string;
  maxConcurrent: number;
  maxPerHost: number;
  scheduleEnabled: boolean;
  scheduleStart: string;
  scheduleEnd: string;
  scheduleDays: number;
  autostart: boolean;
}

export async function fetchEngineInfo(): Promise<EngineInfo | null> {
  try {
    return await invoke<EngineInfo>("engine_info");
  } catch {
    // Browser preview (vite dev without Tauri).
    return null;
  }
}

export async function getMediaTools(): Promise<MediaTools> {
  return invoke<MediaTools>("get_media_tools");
}

export async function checkForUpdates(): Promise<UpdateStatus> {
  return invoke<UpdateStatus>("check_for_updates");
}

export async function expandUrls(input: string): Promise<string[]> {
  return invoke<string[]>("expand_urls", { input });
}

export async function listDownloads(): Promise<EngineDownload[]> {
  return invoke<EngineDownload[]>("list_downloads");
}

export async function addDownload(options: {
  url: string;
  name?: string;
  directory?: string;
  connections?: number;
  startNow: boolean;
}): Promise<string> {
  return invoke<string>("add_download", options);
}

export async function addDownloads(options: {
  urls: string[];
  directory?: string;
  connections?: number;
}): Promise<BatchResult> {
  return invoke<BatchResult>("add_downloads", options);
}

export async function clearFinished(): Promise<number> {
  return invoke<number>("clear_finished");
}

export async function startDownload(id: string): Promise<void> {
  await invoke("start_download", { id });
}

export async function pauseDownload(id: string): Promise<void> {
  await invoke("pause_download", { id });
}

export async function cancelDownload(id: string): Promise<void> {
  await invoke("cancel_download", { id });
}

export async function removeDownload(
  id: string,
  deleteFile = false,
): Promise<void> {
  await invoke("remove_download", { id, deleteFile });
}

export async function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export async function setSettings(settings: Settings): Promise<void> {
  await invoke("set_settings", { settings });
}

export async function setPriority(id: string, priority: number): Promise<void> {
  await invoke("set_priority", { id, priority });
}

export async function pauseAll(): Promise<void> {
  await invoke("pause_all");
}

export async function startAll(): Promise<void> {
  await invoke("start_all");
}

export async function revealPath(path: string): Promise<void> {
  await invoke("reveal_path", { path });
}
