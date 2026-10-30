import { describe, expect, it } from "vitest";
import { filterDownloads, matchesFilters, type Filter } from "./filters";

const rows = [
  {
    name: "movie.mkv",
    host: "cdn.example.org",
    kind: "video",
    status: "downloading",
  },
  {
    name: "manual.pdf",
    host: "docs.example.org",
    kind: "document",
    status: "queued",
  },
  {
    name: "manual-2.pdf",
    host: "docs.example.org",
    kind: "document",
    status: "completed",
  },
  {
    name: "paused.zip",
    host: "files.example.org",
    kind: "archive",
    status: "paused",
  },
];

describe("matchesFilters", () => {
  it("combines status, category, and query", () => {
    expect(matchesFilters(rows[0], "active", null, "")).toBe(true);
    expect(matchesFilters(rows[1], "active", null, "")).toBe(false);
    expect(matchesFilters(rows[2], "completed", "document", "manual")).toBe(true);
    expect(matchesFilters(rows[2], "completed", "video", "")).toBe(false);
    expect(matchesFilters(rows[3], "active", "archive", "paused")).toBe(true);
    expect(matchesFilters(rows[3], "active", null, "movie")).toBe(false);
  });
});

describe("filterDownloads", () => {
  const cases: Array<[Filter, string | null, string, string[]]> = [
    ["all", null, "", ["movie.mkv", "manual.pdf", "manual-2.pdf", "paused.zip"]],
    ["active", null, "", ["movie.mkv", "paused.zip"]],
    ["queued", null, "", ["manual.pdf"]],
    ["completed", "document", "", ["manual-2.pdf"]],
    ["all", "document", "manual", ["manual.pdf", "manual-2.pdf"]],
    ["all", null, "EXAMPLE.ORG", ["movie.mkv", "manual.pdf", "manual-2.pdf", "paused.zip"]],
    ["all", "document", "movie", []],
  ];

  it.each(cases)("filters %s/%s/%s", (filter, category, query, expected) => {
    expect(
      filterDownloads(rows, filter, category, query).map((row) => row.name),
    ).toEqual(expected);
  });
});
