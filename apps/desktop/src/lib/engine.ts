import { invoke } from "@tauri-apps/api/core";
import type { EngineDownload } from "./downloads";

export interface EngineInfo {
  version: string;
  protocol_version: number;
  os: string;
  arch: string;
}

export interface Settings {
  downloadDir: string;
  connections: number;
  speedLimit: number;
  theme: string;
}

export async function fetchEngineInfo(): Promise<EngineInfo | null> {
  try {
    return await invoke<EngineInfo>("engine_info");
  } catch {
    // Browser preview (vite dev without Tauri).
    return null;
  }
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

export async function startDownload(id: string): Promise<void> {
  await invoke("start_download", { id });
}

export async function pauseDownload(id: string): Promise<void> {
  await invoke("pause_download", { id });
}

export async function cancelDownload(id: string): Promise<void> {
  await invoke("cancel_download", { id });
}

export async function removeDownload(id: string): Promise<void> {
  await invoke("remove_download", { id });
}

export async function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export async function setSettings(settings: Settings): Promise<void> {
  await invoke("set_settings", { settings });
}

export async function revealPath(path: string): Promise<void> {
  await invoke("reveal_path", { path });
}
