# Rules for everyone working in this repository

Jutsu Studio for Windows and Linux: a Tauri 2 port of the native macOS app
[jutsuhq/jutsu-studio](https://github.com/jutsuhq/jutsu-studio), whose source is the specification.
Read [ARCHITECTURE.md](ARCHITECTURE.md) first; it maps every Swift module to its crate here.

## Porting

- Port behavior, not syntax. Each Swift type or rule gets an idiomatic Rust or TypeScript equivalent with the
  same observable behavior, the same limits and constants, and the same user-facing wording (English strings
  are copied exactly from the Swift source and its `.xcstrings` catalogs).
- Port the tests with the code. Every Swift test of a ported rule becomes a Rust or Vitest test asserting the
  same cases. A rule without its tests is not done.
- Nothing is invented. A behavior the Swift source does not have is not added; a macOS-only behavior with no
  equivalent here is named in the pull request instead of being silently dropped.
- Stay inside the paths your task names. Shared files (`Cargo.toml` at the root, `src-tauri/src/lib.rs`,
  `src/app/routes.tsx`) are changed only by the lines your task needs.

## Rust

- Edition 2024. Workspace lints apply to every crate: `clippy::pedantic` warns; `unwrap_used`, `expect_used`,
  `panic`, `dbg_macro`, `todo` and `print_stdout` are denied outside tests; `unsafe_code` is forbidden.
- Logic lives in the `crates/`. They never depend on `tauri`. `src-tauri` is an adapter: a command parses its
  input, calls a crate, maps the error. No business rule is written in `src-tauri` or in TypeScript.
- I/O sits behind traits ("ports") declared in `jutsu-core`; the engine and services take them as generics or
  `Arc<dyn Trait>`, so tests run with fakes, in-memory SQLite and a stub HTTP server. No test calls a real
  Higgsfield host or spends anything.
- Errors: a `thiserror` enum per crate, no `anyhow` in libraries. Commands return `Result<T, AppError>`,
  serialized as `{ kind, message }`.
- Async: tokio. Every command that does I/O is `async`. Blocking or CPU work goes to `spawn_blocking`.
  Long-lived tasks take a `CancellationToken`. A lock held across `.await` is a `tokio::sync::Mutex`,
  otherwise `std::sync::Mutex`.
- Money is never a float: `rust_decimal::Decimal` for dollars, integers for cents and credits.
- Logging is `tracing`. Credentials, tokens and signed URLs are never logged.
- Types that cross IPC derive `serde` and `specta::Type`; the TypeScript bindings in `src/bindings.ts` are
  generated, never edited by hand.

## Frontend

- React 19, TypeScript strict, Vite, Tailwind 4. Data comes from typed commands through TanStack Query and is
  invalidated by Rust events; streaming progress comes through channels into a small Zustand store.
- Design tokens live in `src/design/tokens.css` and mirror the macOS app's `JutsuStudio/DesignSystem/Theme.swift`
  one to one. Dark only. Lime `#D4FF2A` is for controls; the violet-fuchsia-pink gradient is for the brand
  mark, render progress and the done glow, never on a control. No color, radius or spacing value is written
  outside the tokens.
- Screens copy the SwiftUI views' layout, order, labels and states. Shared controls are built once in
  `src/design/` and reused.
- Video is plain `<video>` fed from the app's loopback media server, never the `asset:` protocol (it does not
  play on WebKitGTK).
- A component with real logic (keyboard, selection, money, state) has a Vitest test with mocked IPC
  (`@tauri-apps/api/mocks`); markup and static text are not tested.

## Checks

Run these before opening a pull request; CI runs the same on Linux and Windows:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -j 4 -- -D warnings
cargo test --workspace -j 4
pnpm typecheck && pnpm lint && pnpm test
```

## Never

- Never write an absolute path of your machine into a file. Tests use the system temp dir.
- Never add Python to the repository.
- Never add a prebuilt binary or a new native dependency without saying why in the pull request.
- Never commit credentials. No test calls a real Higgsfield host or spends anything.
