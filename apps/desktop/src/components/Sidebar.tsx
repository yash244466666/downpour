import type { ComponentType, ReactNode } from "react";
import {
  Archive,
  FileText,
  Film,
  FolderOpen,
  Music,
  Package,
  type LucideProps,
} from "lucide-react";
import { formatSpeed } from "../lib/format";
import type { DownloadKind } from "../lib/downloads";
import { cn } from "../lib/utils";
import { Logo } from "./Logo";
import { Sparkline } from "./Sparkline";

type IconComponent = ComponentType<LucideProps>;

const CATEGORIES: Array<{
  label: string;
  kind: DownloadKind;
  icon: IconComponent;
  className: string;
}> = [
  { label: "Videos", kind: "video", icon: Film, className: "text-violet" },
  { label: "Music", kind: "audio", icon: Music, className: "text-rose" },
  { label: "Documents", kind: "document", icon: FileText, className: "text-accent" },
  { label: "Archives", kind: "archive", icon: Archive, className: "text-amber" },
  { label: "Programs", kind: "app", icon: Package, className: "text-success" },
];

export function Sidebar({
  categoryCounts,
  category,
  onCategory,
  totalSpeed,
  preview,
  onOpenFolder,
}: {
  categoryCounts: Partial<Record<DownloadKind, number>>;
  category: DownloadKind | null;
  onCategory: (kind: DownloadKind | null) => void;
  totalSpeed: number;
  preview: boolean;
  onOpenFolder: () => void;
}) {
  return (
    <aside className="hidden w-64 shrink-0 flex-col border-r border-line bg-surface/60 lg:flex">
      <div className="flex h-16 items-center gap-3 px-5">
        <Logo className="h-9 w-9" />
        <div className="min-w-0">
          <div className="text-[15px] font-semibold tracking-tight">Downpour</div>
          <div className="text-[11px] text-muted">Download manager</div>
        </div>
        <span className="ml-auto rounded-full border border-line px-2 py-0.5 text-[10px] text-muted">
          v0.1
        </span>
      </div>

      <nav className="flex-1 overflow-y-auto px-3 py-2">
        <SectionLabel className="mt-2">Categories</SectionLabel>
        <div className="mt-1 space-y-0.5">
          {CATEGORIES.map((item) => {
            const isActive = category === item.kind;
            return (
              <button
                key={item.label}
                type="button"
                aria-pressed={isActive}
                onClick={() => onCategory(isActive ? null : item.kind)}
                className={cn(
                  "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13px] transition-colors",
                  isActive
                    ? "bg-accent-soft font-medium text-accent"
                    : "text-muted hover:bg-surface-2 hover:text-fg",
                )}
              >
                <item.icon className={cn("h-4 w-4 shrink-0", item.className)} />
                <span className="truncate">{item.label}</span>
                <span className="ml-auto rounded-md bg-surface-2 px-1.5 py-0.5 text-[10px] tabular-nums">
                  {categoryCounts[item.kind] ?? 0}
                </span>
              </button>
            );
          })}
        </div>
      </nav>

      <div className="border-t border-line p-3">
        <div className="rounded-xl border border-line bg-surface p-3">
          <div className="flex items-center justify-between text-[11px] text-muted">
            <span>Session throughput</span>
            <span
              className={cn(
                "inline-flex items-center gap-1",
                preview ? "text-warning" : "text-success",
              )}
            >
              <span
                className={cn(
                  "h-1.5 w-1.5 rounded-full",
                  preview ? "bg-warning" : "bg-success",
                )}
              />
              {preview ? "preview" : "live"}
            </span>
          </div>
          <div className="mt-1.5 text-lg font-semibold tabular-nums">
            {formatSpeed(totalSpeed)}
          </div>
          <Sparkline
            values={[8, 14, 11, 18, 16, 24, 21, 29, 26, 34, 31, 38]}
            className="mt-2 h-8 w-full"
          />
        </div>
        <button
          type="button"
          onClick={onOpenFolder}
          className="mt-3 flex w-full items-center gap-2 rounded-lg px-2 py-2 text-[12px] text-muted transition-colors hover:bg-surface-2 hover:text-fg"
        >
          <FolderOpen className="h-4 w-4" />
          Open download folder
        </button>
      </div>
    </aside>
  );
}

function SectionLabel({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "px-2.5 text-[10px] font-semibold uppercase tracking-[0.12em] text-muted/70",
        className,
      )}
    >
      {children}
    </div>
  );
}
