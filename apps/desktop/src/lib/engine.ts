import { invoke } from "@tauri-apps/api/core";

export interface EngineInfo {
  version: string;
  protocol_version: number;
  os: string;
  arch: string;
}

export async function fetchEngineInfo(): Promise<EngineInfo | null> {
  try {
    return await invoke<EngineInfo>("engine_info");
  } catch {
    // Browser preview (vite dev without Tauri) or engine not wired yet.
    return null;
  }
}
