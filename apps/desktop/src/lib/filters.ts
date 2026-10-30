export type Filter = "all" | "active" | "queued" | "completed";

export const FILTER_ORDER: Filter[] = ["all", "active", "queued", "completed"];

export const FILTER_LABELS: Record<Filter, string> = {
  all: "All downloads",
  active: "Active",
  queued: "Queued",
  completed: "Completed",
};
