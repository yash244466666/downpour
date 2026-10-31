import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const LIB = readFileSync(join(HERE, "..", "shared", "lib.js"), "utf8");
const BACKGROUND = readFileSync(join(HERE, "..", "shared", "background.js"), "utf8");

function createHarness({ extensionProtocol = 1, engineDown = false } = {}) {
  const calls = [];
  const listeners = { downloads: [], menu: [], request: [], installed: [] };
  const storage = new Map();

  const chrome = {
    runtime: {
      lastError: null,
      onInstalled: { addListener: (listener) => listeners.installed.push(listener) },
      connectNative(host) {
        calls.push({ host });
        const port = {
          onMessage: { addListener: (listener) => (port.onMessageListener = listener) },
          onDisconnect: { addListener: (listener) => (port.onDisconnectListener = listener) },
          postMessage(message) {
            calls.push({ message });
            let response;
            if (engineDown) {
              response = { error: { message: "engine offline" } };
            } else if (message.method === "ping") {
              response = { result: { ok: true }, extension_protocol: extensionProtocol };
            } else if (message.method === "add") {
              response = { result: { id: "download-1" } };
            } else {
              response = { error: { message: "unknown method" } };
            }
            queueMicrotask(() => port.onMessageListener?.(response));
          },
          disconnect() {
            calls.push({ disconnect: true });
          },
        };
        return port;
      },
    },
    downloads: {
      onCreated: { addListener: (listener) => listeners.downloads.push(listener) },
      cancel: (id, callback) => {
        calls.push({ cancel: id });
        callback?.();
      },
      erase: (query) => calls.push({ erase: query }),
    },
    cookies: {
      getAll: (_query, callback) => callback([{ name: "sid", value: "abc" }]),
    },
    webRequest: {
      onBeforeRequest: { addListener: (listener) => listeners.request.push(listener) },
    },
    storage: {
      session: {
        set: (items) => {
          for (const [key, value] of Object.entries(items)) storage.set(key, value);
        },
        get: (keys, callback) => {
          const result = {};
          for (const key of keys) {
            if (storage.has(key)) result[key] = storage.get(key);
          }
          callback(result);
        },
      },
    },
    contextMenus: {
      create: () => {},
      onClicked: { addListener: (listener) => listeners.menu.push(listener) },
    },
  };

  const sandbox = {
    chrome,
    navigator: { userAgent: "test-agent" },
    setTimeout,
    clearTimeout,
    queueMicrotask,
    console,
  };
  vm.createContext(sandbox);
  vm.runInContext(LIB, sandbox);
  vm.runInContext(BACKGROUND, sandbox);
  return { calls, listeners, storage };
}

function flush() {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

function download(url, filename = "/tmp/file.bin") {
  return {
    id: 7,
    url,
    filename,
    referrer: "https://example.com/page",
  };
}

test("hands an http download to the engine and cancels the browser copy", async () => {
  const harness = createHarness();
  await harness.listeners.downloads[0](download("https://example.com/file.bin"));

  assert.ok(harness.calls.some((call) => call.cancel === 7));
  assert.ok(harness.calls.some((call) => call.erase?.id === 7));
  const add = harness.calls.find((call) => call.message?.method === "add");
  assert.ok(add, "expected an add request");
  assert.equal(add.message.params.url, "https://example.com/file.bin");
  assert.equal(add.message.params.name, "file.bin");
  assert.equal(add.message.params.referer, "https://example.com/page");
  assert.equal(add.message.params.user_agent, "test-agent");
  assert.equal(add.message.params.cookie_header, "sid=abc");
  assert.equal(add.message.params.start_now, true);
});

test("leaves the download alone when the engine is offline", async () => {
  const harness = createHarness({ engineDown: true });
  await harness.listeners.downloads[0](download("https://example.com/file.bin"));
  assert.equal(harness.calls.filter((call) => "cancel" in call).length, 0);
  assert.equal(harness.calls.filter((call) => call.message?.method === "add").length, 0);
});

test("leaves the download alone when the host speaks an older protocol", async () => {
  const harness = createHarness({ extensionProtocol: 0 });
  await harness.listeners.downloads[0](download("https://example.com/file.bin"));
  assert.equal(harness.calls.filter((call) => "cancel" in call).length, 0);
  assert.equal(harness.calls.filter((call) => call.message?.method === "add").length, 0);
});

test("ignores blob and data URLs", async () => {
  const harness = createHarness();
  await harness.listeners.downloads[0](download("blob:https://example.com/id"));
  await harness.listeners.downloads[0](download("data:application/octet-stream;base64,AA=="));
  assert.equal(harness.calls.length, 0);
});

test("remembers media URLs per tab and hands them over from the context menu", async () => {
  const harness = createHarness();
  const mediaUrl = "https://cdn.example/master.m3u8?token=1";
  harness.listeners.request[0]({ tabId: 3, url: mediaUrl });
  assert.equal(harness.storage.get("media:3"), mediaUrl);

  harness.listeners.menu[0]({ menuItemId: "downpour-media" }, { id: 3, url: "https://example.com/watch" });
  await flush();

  const add = harness.calls.find((call) => call.message?.method === "add");
  assert.ok(add, "expected an add request");
  assert.equal(add.message.params.url, mediaUrl);
  assert.equal(add.message.params.referer, "https://example.com/watch");
});

test("does not remember non-media URLs", () => {
  const harness = createHarness();
  harness.listeners.request[0]({ tabId: 4, url: "https://example.com/file.zip" });
  assert.equal(harness.storage.has("media:4"), false);
});
