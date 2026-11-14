// Builds the `jutsu` helper (crates/jutsu-cli) for the app being built, before `tauri dev` and `tauri build`.
//
// A debug build lands in target/debug next to the app's own executable, where the app looks for it. For bundles it
// is also copied to src-tauri/binaries/jutsu-<target triple>, the name Tauri's `externalBin` expects
// (tauri.bundle.conf.json), so the installer puts it next to the executable too. That folder is ignored by git.
//
// Tauri passes TAURI_ENV_DEBUG and TAURI_ENV_TARGET_TRIPLE to these commands; without them it builds a debug helper
// for this computer.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const host = /^host: (.+)$/m.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))?.[1]?.trim();
if (!host) {
  throw new Error("rustc -vV names no host target");
}
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || host;
const debug = process.env.TAURI_ENV_DEBUG !== "false";
const profile = debug ? "debug" : "release";

const cargo = ["build", "-p", "jutsu-cli", "--locked", "-j", process.env.JUTSU_HELPER_JOBS || "4"];
if (!debug) {
  cargo.push("--release");
}
if (triple !== host) {
  cargo.push("--target", triple);
}
execFileSync("cargo", cargo, { cwd: root, stdio: "inherit" });

const extension = triple.includes("windows") ? ".exe" : "";
const built = join(root, "target", ...(triple === host ? [] : [triple]), profile, `jutsu${extension}`);
const binaries = join(root, "src-tauri", "binaries");
mkdirSync(binaries, { recursive: true });
copyFileSync(built, join(binaries, `jutsu-${triple}${extension}`));
console.log(`jutsu helper: ${profile} build for ${triple} is ready`);
