/**
 * Copy text to the clipboard. Webviews sometimes reject the async Clipboard
 * API, so fall back to a hidden textarea and the legacy copy command.
 */
export async function copyText(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      // Fall through to the legacy path.
    }
  }

  const area = document.createElement("textarea");
  area.value = text;
  area.setAttribute("readonly", "");
  area.style.position = "fixed";
  area.style.top = "0";
  area.style.left = "0";
  area.style.opacity = "0";
  document.body.appendChild(area);
  area.select();
  area.setSelectionRange(0, area.value.length);

  const copied = document.execCommand("copy");
  area.remove();
  if (!copied) {
    throw new Error("Could not copy the link.");
  }
}
