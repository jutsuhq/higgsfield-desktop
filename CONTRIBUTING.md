# Contributing

Issues and pull requests are welcome. This app follows the macOS app
[jutsuhq/jutsu-studio](https://github.com/jutsuhq/jutsu-studio): a behavior there is ported here with the same rules,
limits and wording, and a new behavior usually lands there first.

## Setup

Install Rust (the toolchain in `rust-toolchain.toml` is picked up by `rustup`), Node 24, pnpm 10, the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform, and CMake and Clang for
whisper.cpp. On Debian or Ubuntu:

```sh
sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev libclang-dev cmake
```

Then:

```sh
pnpm install
pnpm app
```

A debug build keeps its own data, credentials and output folder (`Videos/Jutsu Studio Desktop Debug`), has no
updater, and writes the TypeScript bindings to `src/bindings.ts`.

`pnpm bundle` downloads the pinned LGPL ffmpeg (`scripts/fetch-ffmpeg.mjs`) and builds the installers.

## Checks

CI runs these on Linux and Windows for every pull request; run them before you open one:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm typecheck && pnpm lint && pnpm test && pnpm build
```

No test calls a real Higgsfield host or spends anything: the providers run against a local stub server.

## Code

[AGENTS.md](AGENTS.md) holds the rules for everyone, people and coding agents alike;
[ARCHITECTURE.md](ARCHITECTURE.md) maps the crates and [docs/patterns.md](docs/patterns.md) shows how a feature
crosses from Rust to React. In short:

- Logic lives in `crates/` and never depends on Tauri. `src-tauri` adapts it to IPC; React draws what Rust computes.
- Money is never a float. Credentials, tokens and signed URLs are never logged and never reach the web view.
- A test protects a rule: a price, a limit, a state, a file never overwritten, a wire format. Markup and static
  text are not tested.
- No new dependency, prebuilt binary or native library without saying why in the pull request.

## Releasing

1. Set the version in `Cargo.toml` (`[workspace.package]`) and `package.json`, and merge to `main`.
2. Push a tag `vX.Y.Z` on that commit. `.github/workflows/release.yml` checks the tag, reruns the checks, builds the
   Windows and Linux installers, signs the update bundles and attaches them with `latest.json` to a draft release.
3. Add the release notes and publish the draft. Installed copies find it at
   `releases/latest/download/latest.json` within a day.

The workflow needs the repository secret `TAURI_SIGNING_PRIVATE_KEY`, the private half of `plugins.updater.pubkey` in
`src-tauri/tauri.conf.json` (and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if the key has one). Without it the update
bundles can't be signed and installed copies won't accept them.
