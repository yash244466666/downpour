import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const LIB = readFileSync(join(HERE, "..", "shared", "lib.js"), "utf8");

const sandbox = {};
vm.createContext(sandbox);
vm.runInContext(LIB, sandbox);
const lib = sandbox.DownpourLib;

test("eligible accepts http and https only", () => {
  assert.equal(lib.eligible("https://example.com/file.bin"), true);
  assert.equal(lib.eligible("http://example.com/file.bin"), true);
  assert.equal(lib.eligible("HTTPS://example.com/file.bin"), true);
  assert.equal(lib.eligible("  https://example.com/file.bin  "), true);
  assert.equal(lib.eligible("ftp://example.com/file.bin"), false);
  assert.equal(lib.eligible("blob:https://example.com/id"), false);
  assert.equal(lib.eligible("data:application/octet-stream;base64,AA=="), false);
  assert.equal(lib.eligible("file:///tmp/file.bin"), false);
  assert.equal(lib.eligible(undefined), false);
});

test("baseName keeps the last path component", () => {
  assert.equal(lib.baseName("/home/user/Downloads/file.bin"), "file.bin");
  assert.equal(lib.baseName("C:\\Users\\me\\file.iso"), "file.iso");
  assert.equal(lib.baseName("https://example.com/a/file.bin?token=1#frag"), "file.bin");
  assert.equal(lib.baseName("file.bin"), "file.bin");
  assert.equal(lib.baseName(""), "");
  assert.equal(lib.baseName(undefined), "");
});

test("cookieHeader joins name/value pairs", () => {
  assert.equal(
    lib.cookieHeader([
      { name: "sid", value: "abc" },
      { name: "theme", value: "dark" },
    ]),
    "sid=abc; theme=dark",
  );
  assert.equal(lib.cookieHeader([]), "");
  assert.equal(lib.cookieHeader(null), "");
  assert.equal(lib.cookieHeader([{ name: "flag" }]), "flag=");
});

test("isMediaUrl matches HLS and DASH manifests", () => {
  assert.equal(lib.isMediaUrl("https://cdn.example/master.m3u8"), true);
  assert.equal(lib.isMediaUrl("https://cdn.example/stream.MPD?token=1"), true);
  assert.equal(lib.isMediaUrl("https://cdn.example/video.mp4"), false);
  assert.equal(lib.isMediaUrl("https://cdn.example/page?next=playlist"), false);
  assert.equal(lib.isMediaUrl(undefined), false);
});
