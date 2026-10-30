import { describe, expect, it } from "vitest";
import {
  fileNameFromUrl,
  formatBytes,
  formatEta,
  formatSpeed,
  guessKind,
  percent,
} from "./format";

describe("formatBytes", () => {
  it("formats zero and small values", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
  });

  it("scales to binary units", () => {
    expect(formatBytes(1024)).toBe("1.0 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
  });
});

describe("formatSpeed", () => {
  it("uses an em dash for idle transfers", () => {
    expect(formatSpeed(0)).toBe("—");
    expect(formatSpeed(2048)).toBe("2.0 KB/s");
  });
});

describe("formatEta", () => {
  it("formats seconds, minutes, and hours", () => {
    expect(formatEta(undefined)).toBe("—");
    expect(formatEta(0)).toBe("—");
    expect(formatEta(45)).toBe("45s");
    expect(formatEta(125)).toBe("2m 5s");
    expect(formatEta(3 * 3600 + 600)).toBe("3h 10m");
  });
});

describe("percent", () => {
  it("clamps to 0..100 and handles unknown size", () => {
    expect(percent(50, 100)).toBe(50);
    expect(percent(200, 100)).toBe(100);
    expect(percent(10, 0)).toBe(0);
  });
});

describe("fileNameFromUrl", () => {
  it("uses the last path segment", () => {
    expect(fileNameFromUrl("https://example.com/a/b/file%20name.zip")).toBe(
      "file name.zip",
    );
    expect(fileNameFromUrl("https://example.com/")).toBe("example.com");
    expect(fileNameFromUrl("not a url")).toBe("download");
  });
});

describe("guessKind", () => {
  it("classifies common extensions", () => {
    expect(guessKind("movie.mkv")).toBe("video");
    expect(guessKind("song.flac")).toBe("audio");
    expect(guessKind("report.pdf")).toBe("document");
    expect(guessKind("backup.tar.gz")).toBe("archive");
    expect(guessKind("image.png")).toBe("image");
    expect(guessKind("unknown.xyz")).toBe("other");
  });
});
