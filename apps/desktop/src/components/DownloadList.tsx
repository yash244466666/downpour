import { ChevronDown, Inbox } from "lucide-react";
import type { ComponentType, ReactNode } from "react";
import {
  Archive,
  ArrowDown,
  ArrowUp,
  File,
  FileText,
  Film,
  FolderOpen,
  Image,
  ListPlus,
  Music,
  Package,
  Pause,
  Play,
  Trash2,
  X,
  type LucideProps,
} from "lucide-react";
import type {
  DownloadKind,
  DownloadRow,
  DownloadStatus,
} from "../lib/downloads";
import { formatBytes, formatEta, formatSpeed, percent } from "../lib/format";
import { cn } from "../lib/utils";
import { FILTER_LABELS, FILTER_ORDER, type Filter } from "../lib/filters";

type IconComponent = ComponentType<LucideProps>;

const KIND_META: Record<DownloadKind, { icon: IconComponent; tile: string }> =
  {
    video: { icon: Film, tile: "text-violet bg-violet/10 border-violet/20" },
    audio: { icon: Music, tile: "text-rose bg-rose/10 border-rose/20" },
    document: {
      icon: FileText,
      tile: "text-accent bg-accent-soft border-accent/20",
    },
    archive: { icon: Archive, tile: "text-amber bg-amber/10 border-amber/20" },
    app: {
      icon: Package,
      tile: "text-success bg-success-soft border-success/20",
    },
    image: { icon: Image, tile: "text-rose bg-rose/10 border-rose/20" },
    other: { icon: File, tile: "text-muted bg-surface-2 border-line" },
  };

const STATUS_META: Record<
  DownloadStatus,
  { label: string; pill: string; dot: string; bar: string }
> = {
  downloading: {
    label: "Downloading",
    pill: "text-accent bg-accent-soft",
    dot: "bg-accent",
    bar: "bg-gradient-to-r from-accent to-accent-2",
  },
  paused: {
    label: "Paused",
    pill: "text-warning bg-warning-soft",
    dot: "bg-warning",
    bar: "bg-warning",
  },
  queued: {
    label: "Queued",
    pill: "text-muted bg-surface-2",
    dot: "bg-muted",
    bar: "bg-muted/40",
  },
  completed: {
    label: "Completed",
    pill: "text-success bg-success-soft",
    dot: "bg-success",
    bar: "bg-success",
  },
  failed: {
    label: "Failed",
    pill: "text-danger bg-danger-soft",
    dot: "bg-danger",
    bar: "bg-danger",
  },
  canceled: {
    label: "Canceled",
    pill: "text-muted bg-surface-2",
    dot: "bg-muted/70",
    bar: "bg-muted/40",
  },
};

export function DownloadList({
  downloads,
  total,
  filter,
  preview,
  onFilter,
  onToggle,
  onCancel,
  onRemove,
  onReveal,
  onPriority,
  onStartAll,
  onPauseAll,
  onBatch,
}: {
  downloads: DownloadRow[];
  total: number;
  filter: Filter;
  preview: boolean;
  onFilter: (filter: Filter) => void;
  onToggle: (id: string) => void;
  onCancel: (id: string) => void;
  onRemove: (id: string) => void;
  onReveal: (download: DownloadRow) => void;
  onPriority: (id: string, delta: number) => void;
  onStartAll: () => void;
  onPauseAll: () => void;
  onBatch: () => void;
}) {
  return (
    <section className="mt-6 overflow-hidden rounded-2xl border border-line bg-surface shadow-sm">
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-4 py-3 sm:px-5">
        <h2 className="text-sm font-semibold">Downloads</h2>
        <span className="rounded-md bg-surface-2 px-1.5 py-0.5 text-[10px] tabular-nums text-muted">
          {total}
        </span>
        {preview && (
          <span className="rounded-md border border-warning/30 bg-warning-soft px-1.5 py-0.5 text-[10px] font-medium text-warning">
            Preview data
          </span>
        )}

        <div
          role="tablist"
          aria-label="Filter downloads"
          className="ml-auto flex items-center gap-1 rounded-lg border border-line bg-surface-2 p-0.5"
        >
          {FILTER_ORDER.map((item) => (
            <button
              key={item}
              type="button"
              role="tab"
              aria-selected={filter === item}
              onClick={() => onFilter(item)}
              className={cn(
                "rounded-md px-2.5 py-1 text-[11px] font-medium transition-colors",
                filter === item
                  ? "bg-surface text-fg shadow-sm"
                  : "text-muted hover:text-fg",
              )}
            >
              {item === "all" ? "All" : FILTER_LABELS[item]}
            </button>
          ))}
        </div>

        <div className="hidden items-center gap-1 sm:flex">
          <button
            type="button"
            onClick={onBatch}
            className="inline-flex items-center gap-1 rounded-lg border border-line px-2.5 py-1.5 text-[11px] text-muted transition-colors hover:text-fg"
          >
            <ListPlus className="h-3.5 w-3.5" />
            Batch
          </button>
          <button
            type="button"
            onClick={onStartAll}
            className="inline-flex items-center gap-1 rounded-lg border border-line px-2.5 py-1.5 text-[11px] text-muted transition-colors hover:text-fg"
          >
            <Play className="h-3.5 w-3.5" />
            Start all
          </button>
          <button
            type="button"
            onClick={onPauseAll}
            className="inline-flex items-center gap-1 rounded-lg border border-line px-2.5 py-1.5 text-[11px] text-muted transition-colors hover:text-fg"
          >
            <Pause className="h-3.5 w-3.5" />
            Pause all
          </button>
        </div>

        <button
          type="button"
          title="Sorting arrives in a later milestone"
          className="hidden items-center gap-1 rounded-lg border border-line px-2.5 py-1.5 text-[11px] text-muted transition-colors hover:text-fg sm:flex"
        >
          Newest
          <ChevronDown className="h-3.5 w-3.5" />
        </button>
      </div>

      {downloads.length === 0 ? (
        <EmptyState queryless={filter === "all"} />
      ) : (
        <div className="divide-y divide-line">
          {downloads.map((download) => (
            <DownloadRow
              key={download.id}
              download={download}
              onToggle={onToggle}
              onCancel={onCancel}
              onRemove={onRemove}
              onReveal={onReveal}
              onPriority={onPriority}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function EmptyState({ queryless }: { queryless: boolean }) {
  return (
    <div className="flex flex-col items-center justify-center px-6 py-16 text-center">
      <div className="grid h-12 w-12 place-items-center rounded-2xl border border-line bg-surface-2 text-muted">
        <Inbox className="h-5 w-5" />
      </div>
      <p className="mt-3 text-sm font-medium">Nothing here yet</p>
      <p className="mt-1 max-w-xs text-xs text-muted">
        {queryless
          ? "Add a URL to start a download."
          : "No downloads match this filter or search."}
      </p>
    </div>
  );
}

function DownloadRow({
  download,
  onToggle,
  onCancel,
  onRemove,
  onReveal,
  onPriority,
}: {
  download: DownloadRow;
  onToggle: (id: string) => void;
  onCancel: (id: string) => void;
  onRemove: (id: string) => void;
  onReveal: (download: DownloadRow) => void;
  onPriority: (id: string, delta: number) => void;
}) {
  const kind = KIND_META[download.kind];
  const status = STATUS_META[download.status];
  const KindIcon = kind.icon;
  const progress =
    download.status === "completed"
      ? 100
      : percent(download.downloaded, download.size);
  const canToggle =
    download.status === "downloading" ||
    download.status === "paused" ||
    download.status === "queued";
  const canCancel = canToggle;
  const canRemove =
    download.status === "completed" ||
    download.status === "failed" ||
    download.status === "canceled";
  const canReveal =
    download.status === "completed" && Boolean(download.finalPath);

  return (
    <div className="group grid grid-cols-1 gap-3 px-4 py-4 transition-colors hover:bg-surface-2/40 sm:px-5 md:grid-cols-[minmax(0,1.5fr)_minmax(170px,1fr)_120px_120px_122px] md:items-center md:gap-4">
      <div className="flex min-w-0 items-center gap-3">
        <div
          className={cn(
            "grid h-10 w-10 shrink-0 place-items-center rounded-xl border",
            kind.tile,
          )}
        >
          <KindIcon className="h-[18px] w-[18px]" />
        </div>
        <div className="min-w-0">
          <div
            className="truncate text-[13px] font-medium"
            title={download.error ?? download.name}
          >
            {download.name}
          </div>
          <div className="mt-0.5 flex items-center gap-1.5 text-[11px] text-muted">
            <span className="truncate">{download.host}</span>
            <span aria-hidden="true">·</span>
            <span className="shrink-0">{download.added}</span>
          </div>
          {download.error && (
            <div className="mt-0.5 truncate text-[11px] text-danger">
              {download.error}
            </div>
          )}
        </div>
      </div>

      <div className="min-w-0">
        <div className="mb-1.5 flex items-center justify-between gap-3 text-[11px]">
          <span className="truncate text-muted">
            {formatBytes(download.downloaded)} of{" "}
            {download.size > 0 ? formatBytes(download.size) : "unknown size"}
          </span>
          <span className="shrink-0 font-medium tabular-nums">
            {Math.round(progress)}%
          </span>
        </div>
        <div
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(progress)}
          aria-label={`${download.name} progress`}
          className="relative h-1.5 overflow-hidden rounded-full bg-surface-2"
        >
          <div
            className={cn(
              "relative h-full overflow-hidden rounded-full transition-[width] duration-500",
              status.bar,
              download.status === "downloading" && "progress-shimmer",
            )}
            style={{ width: `${progress}%` }}
          />
        </div>
        <div className="mt-1.5 flex items-center justify-between gap-2 text-[11px] text-muted md:hidden">
          <span className="truncate">
            {download.status === "downloading"
              ? `${formatSpeed(download.speed)} · ${formatEta(download.eta)} left`
              : `${download.connections} connections`}
          </span>
          <StatusPill status={download.status} />
        </div>
      </div>

      <div className="hidden md:block">
        <div className="text-[13px] font-medium tabular-nums">
          {download.status === "downloading"
            ? formatSpeed(download.speed)
            : "—"}
        </div>
        <div className="mt-0.5 text-[11px] text-muted">
          {download.status === "downloading"
            ? `${formatEta(download.eta)} left · ${download.connections} conn`
            : `${download.connections} conn`}
        </div>
      </div>

      <div className="hidden md:flex md:justify-end">
        <StatusPill status={download.status} />
      </div>

      <div className="flex items-center justify-between md:justify-end md:gap-1">
        <div className="flex items-center gap-1">
          {download.status === "queued" && (
            <>
              <RowButton
                label="Raise queue priority"
                onClick={() => onPriority(download.id, 1)}
              >
                <ArrowUp className="h-4 w-4" />
              </RowButton>
              <RowButton
                label="Lower queue priority"
                onClick={() => onPriority(download.id, -1)}
              >
                <ArrowDown className="h-4 w-4" />
              </RowButton>
            </>
          )}
          <RowButton
            label={
              download.status === "downloading"
                ? "Pause download"
                : "Resume download"
            }
            disabled={!canToggle}
            onClick={() => onToggle(download.id)}
          >
            {download.status === "downloading" ? (
              <Pause className="h-4 w-4" />
            ) : (
              <Play className="h-4 w-4" />
            )}
          </RowButton>
          <RowButton
            label="Cancel download"
            disabled={!canCancel}
            onClick={() => onCancel(download.id)}
          >
            <X className="h-4 w-4" />
          </RowButton>
          <RowButton
            label="Show in folder"
            disabled={!canReveal}
            onClick={() => onReveal(download)}
          >
            <FolderOpen className="h-4 w-4" />
          </RowButton>
          <RowButton
            label="Remove from list"
            disabled={!canRemove}
            onClick={() => onRemove(download.id)}
          >
            <Trash2 className="h-4 w-4" />
          </RowButton>
        </div>
      </div>
    </div>
  );
}

function RowButton({
  children,
  label,
  onClick,
  disabled,
}: {
  children: ReactNode;
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={label}
      title={label}
      className={cn(
        "grid h-8 w-8 place-items-center rounded-lg border border-line text-muted transition-colors",
        disabled
          ? "cursor-not-allowed opacity-35"
          : "hover:bg-surface-2 hover:text-fg",
      )}
    >
      {children}
    </button>
  );
}

function StatusPill({ status }: { status: DownloadStatus }) {
  const meta = STATUS_META[status];
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-full px-2 py-0.5 text-[10px] font-medium",
        meta.pill,
      )}
    >
      <span className={cn("h-1.5 w-1.5 rounded-full", meta.dot)} />
      {meta.label}
    </span>
  );
}
