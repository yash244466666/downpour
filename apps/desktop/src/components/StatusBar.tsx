import { useEffect, useState } from "react";
import { Database, Wifi } from "lucide-react";
import { fetchEngineInfo, type EngineInfo } from "../lib/engine";
import { cn } from "../lib/utils";

export function StatusBar({ preview }: { preview: boolean }) {
  const [engine, setEngine] = useState<EngineInfo | null>(null);

  useEffect(() => {
    let cancelled = false;
    void fetchEngineInfo().then((info) => {
      if (!cancelled) setEngine(info);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <footer className="flex h-9 shrink-0 items-center gap-2 border-t border-line bg-surface px-4 text-[10px] text-muted">
      <span
        className={cn(
          "h-1.5 w-1.5 rounded-full",
          engine ? "bg-success" : "bg-warning",
        )}
      />
      <span className="inline-flex items-center gap-1">
        <Database className="h-3 w-3" />
        {engine
          ? `Engine v${engine.version} · protocol ${engine.protocol_version}`
          : preview
            ? "Engine offline · UI preview"
            : "Engine connected"}
      </span>
      <span className="hidden items-center gap-1 sm:inline-flex">
        <Wifi className="h-3 w-3" />
        {engine ? `${engine.os}/${engine.arch}` : "browser preview"}
      </span>
      <span className="ml-auto hidden sm:inline">
        {preview ? "Preview data" : "Live data"}
      </span>
      <span>Downpour 0.1.0</span>
    </footer>
  );
}
