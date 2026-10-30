// Downpour download take-over.
//
// Every eligible browser download is handed to the local engine through the
// native-messaging host. If the engine is offline the browser download is left
// alone, so the extension never breaks browsing.

const HOST = "com.downpour.nm";
const ENGINE_TIMEOUT_MS = 5000;
const PING_TIMEOUT_MS = 2500;

function callEngine(message, timeoutMs = ENGINE_TIMEOUT_MS) {
  return new Promise((resolve) => {
    let port;
    try {
      port = chrome.runtime.connectNative(HOST);
    } catch (error) {
      resolve({ error: { message: String(error) } });
      return;
    }

    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      try {
        port.disconnect();
      } catch {
        // Port already gone.
      }
      resolve(value);
    };
    const timer = setTimeout(() => finish({ error: { message: "engine timeout" } }), timeoutMs);

    port.onMessage.addListener((message) => finish(message));
    port.onDisconnect.addListener(() =>
      finish({
        error: {
          message:
            (chrome.runtime.lastError && chrome.runtime.lastError.message) ||
            "native host disconnected",
        },
      }),
    );
    port.postMessage(message);
  });
}

async function engineOnline() {
  const response = await callEngine({ method: "ping" }, PING_TIMEOUT_MS);
  return Boolean(response && !response.error);
}

function cookieHeaderFor(url) {
  return new Promise((resolve) => {
    chrome.cookies.getAll({ url }, (cookies) => {
      if (chrome.runtime.lastError) {
        resolve("");
        return;
      }
      resolve(cookies.map((cookie) => `${cookie.name}=${cookie.value}`).join("; "));
    });
  });
}

function baseName(path) {
  return (path || "").split(/[\\/]/).pop() || "";
}

function eligible(url) {
  return /^https?:/i.test(url);
}

async function handOff({ url, filename, referer }) {
  const cookieHeader = await cookieHeaderFor(url);
  return callEngine({
    method: "add",
    params: {
      url,
      name: baseName(filename) || undefined,
      referer: referer || undefined,
      user_agent: navigator.userAgent,
      cookie_header: cookieHeader || undefined,
      start_now: true,
    },
  });
}

async function takeOver({ url, filename, referer }) {
  if (!eligible(url)) return;
  if (!(await engineOnline())) return;
  return handOff({ url, filename, referer });
}

chrome.downloads.onCreated.addListener(async (item) => {
  if (!eligible(item.url)) return; // blob:/data: cannot be handed off
  if (!(await engineOnline())) return; // leave normal browser downloads alone
  chrome.downloads.cancel(item.id, () => chrome.downloads.erase({ id: item.id }));
  await handOff({
    url: item.url,
    filename: item.filename,
    referer: item.referrer || item.initiator,
  });
});

const MEDIA_PATTERN = /\.(m3u8|mpd)(\?|#|$)/i;

// Observational sniffing: remember the last manifest URL per tab so the
// context menu can hand it over even when the player loaded it in script.
chrome.webRequest.onBeforeRequest.addListener(
  (details) => {
    if (details.tabId < 0 || !MEDIA_PATTERN.test(details.url)) return;
    chrome.storage.session.set({ [`media:${details.tabId}`]: details.url });
  },
  { urls: ["<all_urls>"] },
);

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "downpour-link",
    title: "Download with Downpour",
    contexts: ["link", "video", "audio"],
  });
  chrome.contextMenus.create({
    id: "downpour-media",
    title: "Download media with Downpour",
    contexts: ["page", "video", "audio"],
  });
});

chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (info.menuItemId === "downpour-media") {
    if (!tab || tab.id === undefined) return;
    const key = `media:${tab.id}`;
    chrome.storage.session.get([key], (data) => {
      const mediaUrl = data[key];
      if (mediaUrl) {
        void takeOver({ url: mediaUrl, referer: tab.url });
      }
    });
    return;
  }

  const url = info.linkUrl || info.srcUrl;
  if (url) {
    void takeOver({ url, referer: tab && tab.url });
  }
});
