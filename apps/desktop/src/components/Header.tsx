import { useEffect, useRef } from "react";
import type { ReactNode } from "react";
import {
  Moon,
  Plus,
  Search,
  Settings,
  Sun,
} from "lucide-react";
import { formatSpeed } from "../lib/format";
import { FILTER_LABELS, type Filter } from "../lib/filters";
import type { Theme } from "../lib/theme";
import { cn } from "../lib/utils";

interface Counts {
  all: number;
  active: number;
  queued: number;
  completed: number;
}

export function Header({
  filter,
  counts,
  totalSpeed,
  query,
  onQuery,
  theme,
  onToggleTheme,
  onAdd,
  onSettings,
}: {
  filter: Filter;
  counts: Counts;
  totalSpeed: number;
  query: string;
  onQuery: (value: string) => void;
  theme: Theme;
  onToggleTheme: () => void;
  onAdd: () => void;
  onSettings: () => void;
}) {
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        searchRef.current?.focus();
      }
      if (event.key === "Escape" && document.activeElement === searchRef.current) {
        searchRef.current?.blur();
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const isMac =
    typeof navigator !== "undefined" &&
    /mac|iphone|ipad|ipod/i.test(navigator.userAgent);
  const shortcutLabel = isMac ? "⌘K" : "Ctrl K";

  const subtitle =
    filter === "all"
      ? `${counts.all} items · ${formatSpeed(totalSpeed)} aggregate`
      : filter === "active"
        ? `${counts.active} downloading now`
        : filter === "queued"
          ? `${counts.queued} waiting to start`
          : `${counts.completed} finished`;

  return (
    <header className="sticky top-0 z-20 flex h-16 shrink-0 items-center gap-3 border-b border-line bg-bg/80 px-4 backdrop-blur-xl sm:px-5 lg:px-8">
      <div className="min-w-0">
        <h1 className="truncate text-[15px] font-semibold tracking-tight">
          {FILTER_LABELS[filter]}
        </h1>
        <p className="truncate text-[11px] text-muted">{subtitle}</p>
      </div>

      <div className="ml-auto flex items-center gap-1.5 sm:gap-2">
        <div className="relative hidden sm:block">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted" />
          <input
            ref={searchRef}
            value={query}
            onChange={(event) => onQuery(event.target.value)}
            placeholder="Search downloads"
            aria-label="Search downloads"
            className="h-9 w-48 rounded-lg border border-line bg-surface pl-9 pr-12 text-[13px] outline-none transition-colors placeholder:text-muted/70 focus:border-accent/50 focus:ring-2 focus:ring-accent/15 lg:w-64"
          />
          <kbd className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 rounded border border-line px-1.5 py-0.5 text-[10px] text-muted">
            {shortcutLabel}
          </kbd>
        </div>

        <button
          type="button"
          onClick={onAdd}
          className="inline-flex h-9 items-center gap-1.5 rounded-lg bg-accent px-3 text-[13px] font-medium text-white shadow-sm transition-[filter] hover:brightness-110 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
        >
          <Plus className="h-4 w-4" />
          <span className="hidden xs:inline">Add URL</span>
          <span className="xs:hidden">Add</span>
        </button>

        <IconButton
          label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
          onClick={onToggleTheme}
        >
          {theme === "dark" ? (
            <Sun className="h-4 w-4" />
          ) : (
            <Moon className="h-4 w-4" />
          )}
        </IconButton>

        <IconButton label="Settings" onClick={onSettings}>
          <Settings className="h-4 w-4" />
        </IconButton>
      </div>
    </header>
  );
}

function IconButton({
  children,
  label,
  onClick,
  disabled,
}: {
  children: ReactNode;
  label: string;
  onClick?: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "grid h-9 w-9 place-items-center rounded-lg border border-line bg-surface text-muted transition-colors",
        disabled
          ? "cursor-not-allowed opacity-45"
          : "hover:border-line hover:bg-surface-2 hover:text-fg",
      )}
    >
      {children}
    </button>
  );
}
