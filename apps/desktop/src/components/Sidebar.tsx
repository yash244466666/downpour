import type { ComponentType, ReactNode } from "react";
import {
  Activity,
  Archive,
  CircleCheck,
  Clock,
  Download,
  FileText,
  Film,
  FolderOpen,
  Music,
  Package,
  type LucideProps,
} from "lucide-react";
import { formatSpeed } from "../lib/format";
import type { Filter } from "../lib/filters";
import { cn } from "../lib/utils";
import { Logo } from "./Logo";
import { Sparkline } from "./Sparkline";

type IconComponent = ComponentType<LucideProps>;

interface Counts {
  all: number;
  active: number;
  queued: number;
  completed: number;
}

const NAV: Array<{ filter: Filter; label: string; icon: IconComponent }> = [
  { filter: "all", label: "All downloads", icon: Download },
  { filter: "active", label: "Active", icon: Activity },
  { filter: "queued", label: "Queued", icon: Clock },
  { filter: "completed", label: "Completed", icon: CircleCheck },
];

const CATEGORIES: Array<{
  label: string;
  count: number;
  icon: IconComponent;
  className: string;
}> = [
  { label: "Videos", count: 1, icon: Film, className: "text-violet" },
  { label: "Music", count: 1, icon: Music, className: "text-rose" },
  { label: "Documents", count: 1, icon: FileText, className: "text-accent" },
  { label: "Archives", count: 2, icon: Archive, className: "text-amber" },
  { label: "Programs", count: 3, icon: Package, className: "text-success" },
];

export function Sidebar({
  counts,
  filter,
  onFilter,
  totalSpeed,
}: {
  counts: Counts;
  filter: Filter;
  onFilter: (filter: Filter) => void;
  totalSpeed: number;
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
        <SectionLabel>Library</SectionLabel>
        <div className="mt-1 space-y-0.5">
          {NAV.map((item) => {
            const isActive = filter === item.filter;
            return (
              <button
                key={item.filter}
                type="button"
                onClick={() => onFilter(item.filter)}
                aria-current={isActive ? "page" : undefined}
                className={cn(
                  "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13px] transition-colors",
                  isActive
                    ? "bg-accent-soft font-medium text-accent"
                    : "text-muted hover:bg-surface-2 hover:text-fg",
                )}
              >
                <item.icon className="h-4 w-4 shrink-0" />
                <span className="truncate">{item.label}</span>
                <span
                  className={cn(
                    "ml-auto rounded-md px-1.5 py-0.5 text-[10px] tabular-nums",
                    isActive
                      ? "bg-accent/15 text-accent"
                      : "bg-surface-2 text-muted",
                  )}
                >
                  {counts[item.filter]}
                </span>
              </button>
            );
          })}
        </div>

        <SectionLabel className="mt-6">Categories</SectionLabel>
        <div className="mt-1 space-y-0.5">
          {CATEGORIES.map((category) => (
            <button
              key={category.label}
              type="button"
              className="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-[13px] text-muted transition-colors hover:bg-surface-2 hover:text-fg"
              title="Category filters arrive with the engine"
            >
              <category.icon
                className={cn("h-4 w-4 shrink-0", category.className)}
              />
              <span className="truncate">{category.label}</span>
              <span className="ml-auto rounded-md bg-surface-2 px-1.5 py-0.5 text-[10px] tabular-nums">
                {category.count}
              </span>
            </button>
          ))}
        </div>
      </nav>

      <div className="border-t border-line p-3">
        <div className="rounded-xl border border-line bg-surface p-3">
          <div className="flex items-center justify-between text-[11px] text-muted">
            <span>Session throughput</span>
            <span className="inline-flex items-center gap-1 text-success">
              <span className="h-1.5 w-1.5 rounded-full bg-success" />
              live
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
          className="mt-3 flex w-full items-center gap-2 rounded-lg px-2 py-2 text-[12px] text-muted transition-colors hover:bg-surface-2 hover:text-fg"
          title="Opens once the engine is wired (M1)"
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
