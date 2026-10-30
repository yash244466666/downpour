export type Filter = "all" | "active" | "queued" | "completed";

export const FILTER_ORDER: Filter[] = ["all", "active", "queued", "completed"];

export const FILTER_LABELS: Record<Filter, string> = {
  all: "All downloads",
  active: "Active",
  queued: "Queued",
  completed: "Completed",
};

export function matchesFilters(
  download: { name: string; host: string; kind: string; status: string },
  filter: Filter,
  category: string | null,
  query: string,
): boolean {
  if (category && download.kind !== category) {
    return false;
  }

  const needle = query.trim().toLowerCase();
  const matchesQuery =
    !needle ||
    download.name.toLowerCase().includes(needle) ||
    download.host.toLowerCase().includes(needle);
  if (!matchesQuery) {
    return false;
  }

  switch (filter) {
    case "all":
      return true;
    case "active":
      return download.status === "downloading" || download.status === "paused";
    case "queued":
      return download.status === "queued";
    case "completed":
      return download.status === "completed";
    default:
      return false;
  }
}

export function filterDownloads<T extends {
  name: string;
  host: string;
  kind: string;
  status: string;
}>(downloads: T[], filter: Filter, category: string | null, query: string): T[] {
  return downloads.filter((download) =>
    matchesFilters(download, filter, category, query),
  );
}
