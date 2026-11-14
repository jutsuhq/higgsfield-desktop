// Downloads the pinned LGPL ffmpeg and ffprobe for the bundle, before `tauri build` (`pnpm bundle`).
//
// The builds are BtbN's static LGPL builds of the FFmpeg 8.1 release branch (https://github.com/BtbN/FFmpeg-Builds):
// no GPL or non-free parts, with libopenh264 for H.264, libass for subtitles and the minterpolate filter Smooth
// Motion uses. Each archive is checked against the SHA-256 below, then ffmpeg and ffprobe are copied to
// src-tauri/binaries/ffmpeg-<target triple> and ffprobe-<target triple>, the names Tauri's `externalBin` expects
// (tauri.bundle.conf.json), so the installer puts them next to the app's executable, where the app looks first.
// That folder is ignored by git. An archive already unpacked for the same build is not downloaded again.
//
// The target is FFMPEG_TARGET_TRIPLE, else TAURI_ENV_TARGET_TRIPLE, else this computer's. BtbN publishes no macOS
// build; the Swift app is the macOS app.
//
// note: BtbN keeps autobuild releases for a limited time; when the pinned tag is gone, pick a newer autobuild of the
// same release branch and put its URLs and SHA-256s here (GitHub lists each asset's digest).

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  createWriteStream,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
  chmodSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";

const RELEASE = "https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-11-09-12-42";
const BUILD = "ffmpeg-n8.1.3-34-ga35157abb9";

/** Per target triple: the archive, its size and SHA-256. */
const ARCHIVES = {
  "x86_64-pc-windows-msvc": {
    name: `${BUILD}-win64-lgpl-8.1`,
    extension: ".zip",
    size: 167646633,
    sha256: "7bdec3860d33145a7cd68146eb8e1414e8881cac76205401211fbdad38ef0ed3",
  },
  "x86_64-unknown-linux-gnu": {
    name: `${BUILD}-linux64-lgpl-8.1`,
    extension: ".tar.xz",
    size: 134831080,
    sha256: "70c3029eaa46cb83746ed5a218bbffbdc4fc3719e4080010af3a9690ff4e65f6",
  },
  "aarch64-unknown-linux-gnu": {
    name: `${BUILD}-linuxarm64-lgpl-8.1`,
    extension: ".tar.xz",
    size: 115525112,
    sha256: "a05bc184dbcacb3ee400ead7a6f04c774bd5be9573fbdc16da73aeaeef9e57c4",
  },
};

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const host = /^host: (.+)$/m.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))?.[1]?.trim();
const triple = process.env.FFMPEG_TARGET_TRIPLE || process.env.TAURI_ENV_TARGET_TRIPLE || host;
const archive = triple ? ARCHIVES[triple] : undefined;
if (!archive) {
  throw new Error(
    `No pinned LGPL ffmpeg build for ${triple ?? "this computer"}; pinned: ${Object.keys(ARCHIVES).join(", ")}`,
  );
}

const binaries = join(root, "src-tauri", "binaries");
const extension = triple.includes("windows") ? ".exe" : "";
const outputs = ["ffmpeg", "ffprobe"].map((tool) => [tool, join(binaries, `${tool}-${triple}${extension}`)]);
const stamp = join(binaries, `ffmpeg-${triple}.build`);
if (
  outputs.every(([, path]) => existsSync(path)) &&
  existsSync(stamp) &&
  readFileSync(stamp, "utf8") === archive.sha256
) {
  console.log(`ffmpeg: ${archive.name} is ready for ${triple}`);
  process.exit(0);
}

const work = mkdtempSync(join(tmpdir(), "jutsu-ffmpeg-"));
try {
  const file = join(work, `${archive.name}${archive.extension}`);
  const url = `${RELEASE}/${archive.name}${archive.extension}`;
  console.log(`ffmpeg: downloading ${url}`);
  const response = await fetch(url);
  if (!response.ok || !response.body) {
    throw new Error(`Downloading ${url} failed: HTTP ${response.status}`);
  }
  await pipeline(Readable.fromWeb(response.body), createWriteStream(file));
  const bytes = readFileSync(file);
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  if (bytes.length !== archive.size || sha256 !== archive.sha256) {
    throw new Error(`${archive.name} isn't the pinned build: ${bytes.length} bytes, SHA-256 ${sha256}`);
  }
  const unpacked = join(work, "unpacked");
  mkdirSync(unpacked);
  if (archive.extension === ".zip" && process.platform !== "win32") {
    execFileSync("unzip", ["-q", file, "-d", unpacked], { stdio: "inherit" });
  } else {
    execFileSync("tar", ["-xf", file, "-C", unpacked], { stdio: "inherit" });
  }
  mkdirSync(binaries, { recursive: true });
  for (const [tool, path] of outputs) {
    copyFileSync(join(unpacked, archive.name, "bin", `${tool}${extension}`), path);
    chmodSync(path, 0o755);
  }
  writeFileSync(stamp, archive.sha256);
  console.log(`ffmpeg: ${archive.name} is ready for ${triple}`);
} finally {
  rmSync(work, { recursive: true, force: true });
}
