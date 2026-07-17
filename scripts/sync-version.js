/**
 * sync-version.js
 *
 * Single source of truth: package.json
 * Syncs the "version" field to:
 *   - src-tauri/Cargo.toml
 *   - src-tauri/tauri.conf.json
 *
 * Usage:  node scripts/sync-version.js
 *         npm run version:sync
 */

import { readFileSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = join(__dirname, "..");

// ── Read version from package.json ──────────────────────────────────
const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf-8"));
const version = pkg.version;

if (!version || !/^\d+\.\d+\.\d+/.test(version)) {
  console.error(`Invalid version in package.json: "${version}"`);
  process.exit(1);
}

console.log(`Syncing version ${version} …`);

// ── 1. Sync tauri.conf.json ─────────────────────────────────────────
const tauriConfPath = join(root, "src-tauri", "tauri.conf.json");
const tauriConf = JSON.parse(readFileSync(tauriConfPath, "utf-8"));
tauriConf.version = version;
writeFileSync(tauriConfPath, JSON.stringify(tauriConf, null, 2) + "\n");
console.log(`  ✔ src-tauri/tauri.conf.json  →  ${version}`);

// ── 2. Sync Cargo.toml ──────────────────────────────────────────────
const cargoPath = join(root, "src-tauri", "Cargo.toml");
let cargo = readFileSync(cargoPath, "utf-8");

const versionRegex = /^(version\s*=\s*)"[^"]+"/m;
if (!versionRegex.test(cargo)) {
  console.error("Could not find version field in Cargo.toml");
  process.exit(1);
}

cargo = cargo.replace(versionRegex, `$1"${version}"`);
writeFileSync(cargoPath, cargo, "utf-8");
console.log(`  ✔ src-tauri/Cargo.toml       →  ${version}`);

console.log("\nAll files are in sync ✅");