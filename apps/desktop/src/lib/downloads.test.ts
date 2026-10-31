import { describe, expect, it } from "vitest";
import {
  formatRelativeTime,
  hostFromUrl,
  rowFromEngine,
  statusFromState,
  type EngineDownload,
} from "./downloads";

const base: EngineDownload = {
  id: "0199a0b1-0000-7000-8000-000000000000",
  url: "https://example.com/files/report.pdf",
  name: "report.pdf",
  directory: "/home/user/Downloads",
  state: "downloading",
  totalBytes: 1000,
  downloadedBytes: 250,
  speedBps: 125,
  connections: 8,
  etaSeconds: 6,
  error: null,
  finalPath: "/home/user/Downloads/report.pdf",
  createdAtMs: 1_700_000_000_000,
  completedAtMs: null,
  priority: 0,
};

describe("statusFromState", () => {
  it("collapses engine states into UI states", () => {
    expect(statusFromState("probing")).toBe("downloading");
    expect(statusFromState("finalizing")).toBe("downloading");
    expect(statusFromState("downloading")).toBe("downloading");
    expect(statusFromState("paused")).toBe("paused");
    expect(statusFromState("completed")).toBe("completed");
    expect(statusFromState("canceled")).toBe("canceled");
    expect(statusFromState("failed")).toBe("failed");
    expect(statusFromState("bogus")).toBe("failed");
  });
});

describe("hostFromUrl", () => {
  it("extracts the host or falls back", () => {
    expect(hostFromUrl("https://cdn.example.org/a.zip")).toBe("cdn.example.org");
    expect(hostFromUrl("nonsense")).toBe("direct link");
  });
});

describe("rowFromEngine", () => {
  it("maps engine fields to the table row", () => {
    const row = rowFromEngine(base, 1_700_000_120_000);
    expect(row.url).toBe(base.url);
    expect(row.name).toBe("report.pdf");
    expect(row.kind).toBe("document");
    expect(row.host).toBe("example.com");
    expect(row.size).toBe(1000);
    expect(row.downloaded).toBe(250);
    expect(row.speed).toBe(125);
    expect(row.status).toBe("downloading");
    expect(row.eta).toBe(6);
    expect(row.added).toBe("2 min ago");
    expect(row.finalPath).toBe("/home/user/Downloads/report.pdf");
    expect(row.priority).toBe(0);
  });

  it("uses the completion time for completed rows", () => {
    const row = rowFromEngine(
      {
        ...base,
        state: "completed",
        downloadedBytes: 1000,
        completedAtMs: 1_700_000_060_000,
      },
      1_700_000_120_000,
    );
    expect(row.status).toBe("completed");
    expect(row.added).toBe("1 min ago");
  });
});

describe("formatRelativeTime", () => {
  const now = 1_700_000_000_000;
  it("formats recent and older timestamps", () => {
    expect(formatRelativeTime(now - 5_000, now)).toBe("just now");
    expect(formatRelativeTime(now - 10 * 60_000, now)).toBe("10 min ago");
    expect(formatRelativeTime(now - 5 * 3_600_000, now)).toBe("5 hr ago");
    expect(formatRelativeTime(now - 26 * 3_600_000, now)).toBe("yesterday");
  });
});
