# Patterns

How a feature crosses from Rust to React. The connection feature follows every one of these; read
`crates/jutsu-app/src/connection/`, `src-tauri/src/commands/connection.rs` and `src/app/liveData.ts` as the worked
example.

The rule underneath all of them: business rules live in `jutsu-app` (or the crates below it), `src-tauri` adapts
them to IPC, and React draws what it is given. When the UI needs a computed value (a plate, a row, an alert),
Rust computes it and sends it as a type.

## The composition root

`jutsu_app::AppEnvironment` owns the store, the file layout, the preferences, the connection store and the active
provider. It knows nothing of Tauri: `AppEnvironment::open(AppDirectories, PlatformServices)` takes its folders and
its platform services (credential vault, network services, provider factory) as arguments, so a test builds one in a
temp dir with fakes (`environment.rs` tests). `src-tauri/src/lib.rs` builds the real one in `setup` and manages it
as `Arc<AppEnvironment>`.

## Add a command

1. Put the logic in `jutsu-app` as a method that returns a serializable type. Types that cross IPC derive
   `serde::Serialize` (and `Deserialize` when they come in) and `specta::Type`, with
   `#[serde(rename_all = "camelCase")]`. Avoid `i64`/`u64` in them: specta refuses BigInt-style types, so use `i32`
   or a preformatted string.
2. Add an async command in `src-tauri/src/commands/<feature>.rs`:

   ```rust
   #[tauri::command]
   #[specta::specta]
   pub(crate) async fn connection_status(environment: Environment<'_>) -> Result<ConnectionSnapshot, AppError> {
       Ok(environment.connection_snapshot())
   }
   ```

   It parses its input, calls `jutsu-app` and maps the error. If a new error needs its own kind, add it to
   `AppError` and `AppErrorKind` in `src-tauri/src/error.rs` with a `From` impl.
3. List it in `collect_commands!` in `specta_builder()` (`src-tauri/src/lib.rs`) and in the bindings test there.
4. Regenerate `src/bindings.ts`: a debug run (`pnpm tauri dev`) writes it. Never edit it by hand.
5. Mock it in tests through `mockCommands({ connection_status: () => ... })` (`src/lib/testing/ipcMock.ts`), keyed by
   the snake_case name `invoke` receives.

## Add an event

Events say "something changed"; they carry the new value when it is small and cheap.

1. Define it in `src-tauri/src/events.rs`:

   ```rust
   #[derive(Debug, Clone, Serialize, specta::Type, Event)]
   pub struct ConnectionChanged { pub snapshot: ConnectionSnapshot }
   ```

2. List it in `collect_events!` in `specta_builder()`. `builder.mount_events(app)` in `setup` registers it.
3. Send it from a forwarder in `events::forward`: subscribe to the `jutsu-app` state's `tokio::sync::watch` channel
   (`ConnectionStore::subscribe`, `PreferencesStore::subscribe`) and emit on every change. `jutsu-app` never emits
   itself; it only bumps its watch channel.
4. In React, add it to `useLiveEvents()` in `src/app/liveData.ts` (mounted once by `RootLayout`).

## Add a query hook

1. Add the key to `src/lib/queryKeys.ts`. Keys are paths: invalidating `["connection"]` also covers
   `["connection", "details"]`.
2. Wrap the command with `useCommand(key, () => commands.x())` (`src/lib/ipc.ts`). The query client keeps results
   fresh until an event says otherwise (`staleTime: Infinity`, no refetch on focus, no retries).
3. Keep it fresh from Rust with `useInvalidateOn(events.x, keys, update?)` (`src/lib/events.ts`). `update` writes
   the payload straight into the cache (`setQueryData`) when the event carries the new value; `keys` are fetched
   again. `useAppEvent` underneath always unlistens on unmount, also before `listen` has resolved.
4. For changes, `useCommandMutation((args) => commands.y(args), keysToInvalidate)`. State an event already reports
   needs no keys here.
5. Test with `renderRoute(path, handlers)` (`src/test/renderRoute.tsx`) or `mockCommands` plus a fresh
   `createQueryClient()`; drive events with `emitConnection(snapshot)` / `emitEvent(name, payload)`.

## Add a progress channel

Progress of one long call (an export, a download) goes through a `tauri::ipc::Channel<T>` argument, not an event:
it belongs to that call only and closes with it.

```rust
#[derive(Clone, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportProgress { Started { total: i32 }, Step { done: i32 }, Finished }

#[tauri::command]
#[specta::specta]
pub(crate) async fn export(environment: Environment<'_>, on_progress: Channel<ExportProgress>) -> Result<(), AppError> {
    // jutsu-app takes a plain callback (`impl Fn(ExportProgress)`); the adapter turns it into channel sends.
    environment.export(|progress| { let _ = on_progress.send(progress); }).await?;
    Ok(())
}
```

The generated binding takes a `Channel<ExportProgress>` from `@tauri-apps/api/core`:
`const channel = new Channel<ExportProgress>(); channel.onmessage = setProgress; await commands.export(channel);`.
Keep the latest message in component state; there is nothing to cache.

## Media files

`<video>` and `<img>` load files through the loopback media server (`jutsu_app::MediaServer`), never `asset:`.
`commands.mediaUrl(path)` returns a tokenized `http://127.0.0.1:<port>/...` URL for a file under a registered folder
(`AppEnvironment::media_roots`) and refuses anything else. The CSP allows `http://127.0.0.1:*` for `img-src` and
`media-src` only.
