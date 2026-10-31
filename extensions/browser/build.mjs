#!/usr/bin/env node
// Assemble per-browser extension bundles from the shared sources.
//
//   node extensions/browser/build.mjs                 assemble dist/<browser>/
//   node extensions/browser/build.mjs --check         validate, then assemble
//   node extensions/browser/build.mjs --package DIR   also write release zips
//
// No npm dependencies. Packaging shells out to the system `zip` command,
// which the release workflow runs on Ubuntu.

import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(HERE, "../..");
const DIST = join(HERE, "dist");
const SHARED = ["lib.js", "background.js"];
const TAURI_CONF = join(REPO_ROOT, "apps/desktop/src-tauri/tauri.conf.json");
const CLI_SOURCE = join(REPO_ROOT, "apps/cli/src/main.rs");

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    fail(`cannot read ${path}: ${error.message}`);
  }
}

function parseArgs(argv) {
  const options = { check: false, packageDir: null };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--check") {
      options.check = true;
    } else if (argument === "--package") {
      const value = argv[index + 1];
      if (!value) fail("--package needs an output directory");
      options.packageDir = resolve(value);
      index += 1;
    } else if (argument === "--help" || argument === "-h") {
      console.log("usage: build.mjs [--check] [--package DIR]");
      process.exit(0);
    } else {
      fail(`unknown argument: ${argument}`);
    }
  }
  return options;
}

function browserDirs() {
  return readdirSync(HERE, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .filter((name) => !["shared", "test", "dist"].includes(name))
    .filter((name) => existsSync(join(HERE, name, "manifest.json")))
    .sort();
}

function validateShared() {
  for (const file of SHARED) {
    const path = join(HERE, "shared", file);
    if (!existsSync(path)) fail(`missing shared file: ${path}`);
  }
}

function validateManifest(name, manifest, extensionVersion) {
  if (manifest.manifest_version !== 3) {
    fail(`${name}: manifest_version must be 3`);
  }
  if (manifest.version !== "0.0.0") {
    fail(`${name}: source manifest version must be the "0.0.0" placeholder`);
  }
  if (manifest.name !== "Downpour") {
    fail(`${name}: unexpected name ${JSON.stringify(manifest.name)}`);
  }
  if (!Array.isArray(manifest.permissions) || !manifest.permissions.includes("nativeMessaging")) {
    fail(`${name}: nativeMessaging permission is required`);
  }
  const hosts = manifest.host_permissions ?? [];
  if (!hosts.includes("http://*/*") || !hosts.includes("https://*/*")) {
    fail(`${name}: host_permissions must cover http and https`);
  }

  if (name === "chromium") {
    if (!manifest.background || manifest.background.service_worker !== "background.js") {
      fail("chromium: background.service_worker must be background.js");
    }
  } else if (name === "firefox") {
    const scripts = manifest.background?.scripts;
    if (!Array.isArray(scripts)) {
      fail("firefox: background.scripts is required (MV3 event page)");
    }
    const libIndex = scripts.indexOf("lib.js");
    const backgroundIndex = scripts.indexOf("background.js");
    if (libIndex === -1 || backgroundIndex === -1 || libIndex > backgroundIndex) {
      fail("firefox: lib.js must be listed before background.js");
    }
    const geckoId = manifest.browser_specific_settings?.gecko?.id;
    if (!geckoId) fail("firefox: browser_specific_settings.gecko.id is required");
    const cli = readFileSync(CLI_SOURCE, "utf8");
    const constant = cli.match(/const FIREFOX_EXTENSION_ID: &str = "([^"]+)";/);
    if (constant && constant[1] !== geckoId) {
      fail(`firefox: gecko id ${geckoId} does not match the CLI constant ${constant[1]}`);
    }
  }

}

function checkSyntax(files) {
  for (const file of files) {
    const result = spawnSync(process.execPath, ["--check", file], { encoding: "utf8" });
    if (result.status !== 0) {
      fail(`${file}: ${result.stderr.trim() || "syntax check failed"}`);
    }
  }
}

function assemble(browsers, manifests, extensionVersion) {
  rmSync(DIST, { recursive: true, force: true });
  mkdirSync(DIST, { recursive: true });

  for (const name of browsers) {
    const target = join(DIST, name);
    mkdirSync(target, { recursive: true });
    for (const file of SHARED) {
      const source = readFileSync(join(HERE, "shared", file));
      writeFileSync(join(target, file), source);
    }
    const stamped = { ...manifests[name], version: extensionVersion };
    writeFileSync(join(target, "manifest.json"), `${JSON.stringify(stamped, null, 2)}\n`);
    console.log(`built ${name} ${extensionVersion} -> ${target}`);
  }
}

function packageExtensions(browsers, extensionVersion, outputDir) {
  mkdirSync(outputDir, { recursive: true });
  for (const name of browsers) {
    const target = join(outputDir, `downpour-${name}-${extensionVersion}.zip`);
    const result = spawnSync("zip", ["-qrX", target, "."], {
      cwd: join(DIST, name),
      encoding: "utf8",
    });
    if (result.error && result.error.code === "ENOENT") {
      fail("the `zip` command is required for --package");
    }
    if (result.status !== 0) {
      fail(`zip failed for ${name}: ${result.stderr.trim()}`);
    }
    console.log(`packaged ${name} -> ${target}`);
  }
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  validateShared();

  const appVersion = readJson(TAURI_CONF).version;
  const extensionVersion = String(appVersion).split("-")[0];
  if (!/^\d+(\.\d+){0,3}$/.test(extensionVersion)) {
    fail(`app version ${appVersion} cannot be used as an extension version`);
  }

  const browsers = browserDirs();
  for (const required of ["chromium", "firefox"]) {
    if (!browsers.includes(required)) fail(`missing browser directory: ${required}`);
  }

  const manifests = {};
  for (const name of browsers) {
    manifests[name] = readJson(join(HERE, name, "manifest.json"));
    if (options.check) validateManifest(name, manifests[name], extensionVersion);
  }

  if (options.check) {
    checkSyntax([
      join(HERE, "shared", "lib.js"),
      join(HERE, "shared", "background.js"),
      join(HERE, "build.mjs"),
    ]);
  }

  assemble(browsers, manifests, extensionVersion);
  if (options.packageDir) {
    packageExtensions(browsers, extensionVersion, options.packageDir);
  }
}

main();
