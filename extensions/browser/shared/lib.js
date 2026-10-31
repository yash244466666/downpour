// Pure helpers shared by every Downpour browser build.
//
// This is a classic script, not an ES module: it can be listed first in a
// Firefox event page's `background.scripts` and loaded with `importScripts()`
// from a Chromium MV3 service worker. It exposes exactly one frozen global.

(function (global) {
  "use strict";

  const MEDIA_PATTERN = /\.(m3u8|mpd)(\?|#|$)/i;

  /** True for URLs the engine is allowed to fetch. */
  function eligible(url) {
    return typeof url === "string" && /^https?:/i.test(url.trim());
  }

  /** Last path component, ignoring query strings and fragments. */
  function baseName(path) {
    if (typeof path !== "string") return "";
    const withoutQuery = path.split(/[?#]/)[0];
    return withoutQuery.split(/[\\/]/).pop() || "";
  }

  /** Join a `cookies.getAll` result into a Cookie header value. */
  function cookieHeader(cookies) {
    if (!Array.isArray(cookies)) return "";
    return cookies
      .filter((cookie) => cookie && typeof cookie.name === "string")
      .map((cookie) => `${cookie.name}=${cookie.value ?? ""}`)
      .join("; ");
  }

  /** True for HLS/DASH manifest URLs worth remembering per tab. */
  function isMediaUrl(url) {
    return typeof url === "string" && MEDIA_PATTERN.test(url);
  }

  global.DownpourLib = Object.freeze({
    eligible,
    baseName,
    cookieHeader,
    isMediaUrl,
  });
})(globalThis);
