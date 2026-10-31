import { afterEach, describe, expect, it, vi } from "vitest";
import { copyText } from "./clipboard";

function fallbackDocument(execResult: boolean) {
  const textarea = {
    value: "",
    style: {} as CSSStyleDeclaration,
    setAttribute: vi.fn(),
    select: vi.fn(),
    setSelectionRange: vi.fn(),
    remove: vi.fn(),
  };
  return {
    textarea,
    document: {
      createElement: () => textarea,
      body: { appendChild: vi.fn() },
      execCommand: vi.fn().mockReturnValue(execResult),
    },
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("copyText", () => {
  it("uses the async clipboard when available", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText } });

    await copyText("https://example.com/file.zip");

    expect(writeText).toHaveBeenCalledWith("https://example.com/file.zip");
  });

  it("falls back to a textarea when the async API rejects", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("denied"));
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const fake = fallbackDocument(true);
    vi.stubGlobal("document", fake.document);

    await copyText("https://example.com/file.zip");

    expect(fake.textarea.value).toBe("https://example.com/file.zip");
    expect(fake.textarea.setAttribute).toHaveBeenCalledWith("readonly", "");
    expect(fake.document.body.appendChild).toHaveBeenCalledWith(fake.textarea);
    expect(fake.document.execCommand).toHaveBeenCalledWith("copy");
    expect(fake.textarea.remove).toHaveBeenCalled();
  });

  it("throws when the fallback copy fails", async () => {
    vi.stubGlobal("navigator", {});
    vi.stubGlobal("document", fallbackDocument(false).document);

    await expect(copyText("https://example.com/file.zip")).rejects.toThrow(
      "Could not copy the link.",
    );
  });
});
