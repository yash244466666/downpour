import type { ComponentType } from "react";
import {
  Activity,
  CircleCheck,
  Clock,
  Gauge,
  type LucideProps,
} from "lucide-react";
import type { DemoDownload } from "../data/mock";
import { formatBytes, formatSpeed } from "../lib/format";
import { cn } from "../lib/utils";
import { Sparkline } from "./Sparkline";

type IconComponent = ComponentType<LucideProps>;

export function SummaryCards({ downloads }: { downloads: DemoDownload[] }) {
  const active = downloads.filter((item) => item.status === "downloading");
  const paused = downloads.filter((item) => item.status === "paused").length;
  const queued = downloads.filter((item) => item.status === "queued").length;
  const completed = downloads.filter((item) => item.status === "completed");
  const speed = active.reduce((total, item) => total + item.speed, 0);
  const connections = active.reduce(
    (total, item) => total + item.connections,
    0,
  );
  const completedBytes = completed.reduce(
    (total, item) => total + item.size,
    0,
  );

  return (
    <section className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <StatCard
        label="Active downloads"
        value={String(active.length)}
        hint={paused > 0 ? `${paused} paused` : "All workers running"}
        icon={Activity}
        featured
      />
      <StatCard
        label="Total speed"
        value={formatSpeed(speed)}
        hint={`${connections} connections open`}
        icon={Gauge}
        chart={[9, 15, 12, 21, 18, 27, 24, 31, 28, 36, 33, 41]}
      />
      <StatCard
        label="In queue"
        value={String(queued)}
        hint="Starts automatically"
        icon={Clock}
      />
      <StatCard
        label="Completed today"
        value={String(completed.length)}
        hint={`${formatBytes(completedBytes)} this session`}
        icon={CircleCheck}
      />
    </section>
  );
}

function StatCard({
  label,
  value,
  hint,
  icon: Icon,
  featured = false,
  chart,
}: {
  label: string;
  value: string;
  hint: string;
  icon: IconComponent;
  featured?: boolean;
  chart?: number[];
}) {
  return (
    <div
      className={cn(
        "relative overflow-hidden rounded-2xl border border-line bg-surface p-4 shadow-sm",
        featured &&
          "border-accent/25 bg-gradient-to-br from-accent-soft via-surface to-surface",
      )}
    >
      <div className="flex items-center justify-between">
        <span className="text-[11px] font-medium uppercase tracking-[0.08em] text-muted">
          {label}
        </span>
        <Icon
          className={cn(
            "h-4 w-4",
            featured ? "text-accent" : "text-muted/80",
          )}
        />
      </div>
      <div className="mt-2 text-2xl font-semibold tracking-tight tabular-nums">
        {value}
      </div>
      <div className="mt-0.5 text-[11px] text-muted">{hint}</div>
      {chart && (
        <Sparkline
          values={chart}
          className="pointer-events-none absolute -bottom-1 right-0 h-12 w-32 opacity-60"
        />
      )}
    </div>
  );
}
