import { useEffect, useMemo, useState } from "react";
import type { FormEvent } from "react";
import { ListPlus, X } from "lucide-react";
import * as engine from "../lib/engine";
import { cn } from "../lib/utils";

export function BatchImportDialog({
  onClose,
  onAdd,
}: {
  onClose: () => void;
  onAdd: (urls: string[]) => void;
}) {
  const [text, setText] = useState("");
  const [expanded, setExpanded] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  const valid = useMemo(
    () =>
      (expanded ?? []).filter((url) => /^https?:\/\//i.test(url)),
    [expanded],
  );

  async function preview(event: FormEvent) {
    event.preventDefault();
    if (!text.trim()) {
      setError("Paste at least one URL or pattern.");
      return;
    }
    try {
      const urls = await engine.expandUrls(text);
      if (urls.length === 0) {
        setError("No URLs found in that input.");
        return;
      }
      setError(null);
      setExpanded(urls);
    } catch (cause) {
      setError(String(cause));
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-black/50 p-4 backdrop-blur-sm"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <form
        role="dialog"
        aria-modal="true"
        aria-labelledby="batch-title"
        onSubmit={preview}
        className="animate-toast-in flex max-h-[90vh] w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-2xl"
      >
        <div className="flex items-start gap-3 border-b border-line px-5 py-4">
          <div className="mt-0.5 grid h-9 w-9 place-items-center rounded-xl bg-accent-soft text-accent">
            <ListPlus className="h-4 w-4" />
          </div>
          <div>
            <h2 id="batch-title" className="text-sm font-semibold">
              Batch import
            </h2>
            <p className="mt-0.5 text-[11px] text-muted">
              One URL per line. Patterns like img[001-100].jpg or
              file{"{a,b}"}.zip expand automatically.
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close batch import"
            className="ml-auto grid h-8 w-8 place-items-center rounded-lg text-muted transition-colors hover:bg-surface-2 hover:text-fg"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-5">
          <label className="block">
            <span className="mb-1.5 block text-[11px] font-medium text-muted">
              URLs
            </span>
            <textarea
              autoFocus
              value={text}
              onChange={(event) => {
                setText(event.target.value);
                setExpanded(null);
                if (error) setError(null);
              }}
              rows={6}
              spellCheck={false}
              placeholder={"https://example.com/file1.zip\nhttps://example.com/img[001-010].jpg"}
              className="w-full rounded-lg border border-line bg-surface-2 px-3 py-2 font-mono text-[12px] leading-relaxed outline-none transition-colors placeholder:text-muted/50 focus:border-accent/50 focus:ring-2 focus:ring-accent/15"
            />
          </label>

          {expanded && (
            <div className="rounded-lg border border-line">
              <div className="flex items-center justify-between border-b border-line px-3 py-2 text-[11px] text-muted">
                <span>
                  {expanded.length} expanded · {valid.length} http(s)
                </span>
                {expanded.length !== valid.length && (
                  <span>{expanded.length - valid.length} skipped</span>
                )}
              </div>
              <div className="max-h-48 overflow-y-auto p-2">
                {expanded.slice(0, 200).map((url) => (
                  <div
                    key={url}
                    className={cn(
                      "truncate rounded px-1.5 py-0.5 font-mono text-[11px]",
                      /^https?:\/\//i.test(url)
                        ? "text-fg"
                        : "text-danger line-through",
                    )}
                  >
                    {url}
                  </div>
                ))}
                {expanded.length > 200 && (
                  <div className="px-1.5 py-0.5 text-[11px] text-muted">
                    …and {expanded.length - 200} more
                  </div>
                )}
              </div>
            </div>
          )}

          {error && (
            <p className="text-[11px] text-danger" role="alert">
              {error}
            </p>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-line bg-surface-2/40 px-5 py-3">
          <button
            type="button"
            onClick={onClose}
            className="h-9 rounded-lg border border-line bg-surface px-3 text-[12px] font-medium text-muted transition-colors hover:text-fg"
          >
            Cancel
          </button>
          {expanded ? (
            <button
              type="button"
              onClick={() => onAdd(valid)}
              disabled={valid.length === 0}
              className="h-9 rounded-lg bg-accent px-4 text-[12px] font-semibold text-white shadow-sm transition-[filter] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-50"
            >
              Add {valid.length} downloads
            </button>
          ) : (
            <button
              type="submit"
              className="h-9 rounded-lg bg-accent px-4 text-[12px] font-semibold text-white shadow-sm transition-[filter] hover:brightness-110"
            >
              Preview
            </button>
          )}
        </div>
      </form>
    </div>
  );
}
