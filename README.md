<div align="center">

<img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Jutsu Studio icon">

# Higgsfield Desktop App for Windows and Linux (Jutsu Studio)

**The open-source Higgsfield desktop app for Windows and Linux. Unofficial.**
Make video and images with Genjutsu, Seedance, Kling and Higgsfield's other models in a desktop studio, and keep
every file on your disk.

[![Download](https://img.shields.io/badge/Download-Windows%20%7C%20Linux-8b5cf6?style=for-the-badge)](https://github.com/jutsuhq/higgsfield-desktop/releases/latest)

[Quick start](#quick-start) · [Features](https://github.com/jutsuhq/jutsu-studio/blob/main/docs/features.md) · [macOS app](https://github.com/jutsuhq/jutsu-studio) · [Website](https://jutsustudio.com/)

[![License: MIT](https://img.shields.io/badge/License-MIT-8b5cf6.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-0078D6)](#quick-start)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
[![Unofficial](https://img.shields.io/badge/unofficial-not%20affiliated%20with%20Higgsfield-e879f9)](#disclaimer)

</div>

---

Jutsu Studio started as a native Mac app ([jutsuhq/jutsu-studio](https://github.com/jutsuhq/jutsu-studio)). This is
the same app for Windows and Linux, built with Tauri 2, Rust and React: the same screens, the same models, the same
rules for prices and budgets, the same library on disk.

## Why Jutsu Studio

Higgsfield's web app is made for one generation at a time in a browser tab. Jutsu Studio is a desktop app made for
working through a project: many clips, many takes, and a library you keep.

- **Your files, on your disk.** Every result is downloaded to `Videos/Jutsu Studio` as soon as it's ready, with its
  source, references, prompt and cost. Higgsfield's links expire; your library doesn't, and it opens offline.
- **Any video, straight from your file manager.** Drop a video file or paste a link. The app transcodes and trims it
  on your computer with ffmpeg and uploads only the range you pick, so you never pay for seconds you cut.
- **Close the window, not the work.** Queue as many jobs as you like. They keep running in the background, show in
  the system tray, notify you when they're done, and pick up again after a restart or a dropped connection, without
  ever paying twice.
- **Built for many takes.** Cut several ranges from one clip in one go, make cheap 480p drafts before the final,
  compare takes side by side, and keep a character, preset and prompt as a template.
- **The price before the click.** Every button shows what the job costs, and an optional daily cap guards your budget.
- **Straight into your editor.** Export for Premiere Pro or DaVinci Resolve, with take names that sort in order.
- **Free work on your computer.** Upscale with Real-ESRGAN, smooth motion with ffmpeg and make subtitles with
  whisper.cpp, all locally and at no cost.
- **Agents welcome.** Claude Code and other MCP clients can quote, make, follow and export jobs within a daily
  budget you set, through the bundled `jutsu mcp` helper.
- **Nothing in between.** No Jutsu server, no account of ours, no markup, no telemetry. The app talks to Higgsfield
  directly with your API key or your Higgsfield sign-in and keeps the credentials in your system's credential store.

## What it does

- **Genjutsu:** motion transfer, object swap and restyle of any video.
- **Seedance 2.5:** motion transfer, a clip from a prompt and images alone, start and end frames, and Edit or Extend
  of an existing video.
- **Kling 4.0 and Kling 4.0 Flash:** a clip from a prompt, start and end frames or references, and Edit of an
  existing video.
- **Gemini Omni Flash 1.2** and **Higgsfield Intone** on a Higgsfield account, plus Kling 3.0 Motion Control,
  Kling O3, Wan 3.0, Cinema Studio 4.0 and the other models the macOS app offers.
- **Images** with Higgsfield's image models, characters and styles.
- **On a Higgsfield account:** Upscale, Reframe, Remove Background, Dub and Change Voice for a result; the Motion
  Library; your reference elements; your plan's unlimited generations.

The [macOS app's feature list](https://github.com/jutsuhq/jutsu-studio/blob/main/docs/features.md) applies here
too, with the differences below.

### Differences from the macOS app

| macOS app              | Windows and Linux                                                                        |
| ---------------------- | ---------------------------------------------------------------------------------------- |
| macOS Keychain         | Windows Credential Manager, or the Secret Service (GNOME Keyring, KWallet) on Linux      |
| Menu bar extra         | System tray icon and panel                                                               |
| AVFoundation           | ffmpeg, bundled with the installers, for trimming, transcoding, frames and smooth motion |
| Upscale on This Mac    | Real-ESRGAN ncnn Vulkan, downloaded once when you ask                                    |
| Apple Speech, Parakeet | whisper.cpp with Whisper Small, downloaded once when you ask                             |
| Sparkle                | Tauri's updater, from this repository's releases                                         |
| Share sheet            | Show in File Manager                                                                     |
| Copy Images            | Copy Image copies one picture at a time                                                  |

## Quick start

**1. Install.** Grab the installer for your system from the
[latest release](https://github.com/jutsuhq/higgsfield-desktop/releases/latest):

- Windows 10 or 11: `Jutsu.Studio_<version>_x64-setup.exe`
- Linux: the `.AppImage`, or the `.deb` (Debian, Ubuntu) or `.rpm` (Fedora, openSUSE) package

On Linux the system tray needs an AppIndicator host (built into KDE and most desktops; on GNOME, the AppIndicator
extension).

**2. Connect Higgsfield.** Either sign in with your Higgsfield account to spend your plan's credits (OAuth in your
browser through Higgsfield's MCP server; the app never sees your password), or paste an API key from the
[Higgsfield console](https://console.higgsfield.ai) to pay as you go. Generations are billed to **your own
Higgsfield account**; Jutsu Studio is free and takes no cut.

**3. Drop a video.** Pick a 4–30 s range, add a character image (or choose a restyle preset), check the price tag and
hit **Draft 480p**. A couple of minutes later the result is in your library and in `Videos/Jutsu Studio`.

## Privacy & safety

- No backend, no accounts of ours, no telemetry. The app talks straight to Higgsfield with your key or sign-in, and
  to GitHub and Hugging Face for updates and for the engines you choose to download.
- Bring your own account: no key ships with the app. Your API key or sign-in tokens are stored only in your system's
  credential store; logs redact credentials and signed URLs.
- Generation runs on Higgsfield's servers. The source video, any reference images and the prompt are uploaded to
  Higgsfield for each job, and the results are downloaded back.
- Your library, history and settings stay on your computer.
- Found a vulnerability? Report it privately as described in [SECURITY.md](SECURITY.md).

## Build from source

Requires Rust (stable, pinned by `rust-toolchain.toml`), Node 24 and pnpm 10, the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform, and CMake and Clang
(whisper.cpp is built from source).

```sh
pnpm install
pnpm app          # run a debug build
pnpm bundle       # fetch ffmpeg and build the installers
```

[CONTRIBUTING.md](CONTRIBUTING.md) has the checks, the code rules and how releases are made;
[ARCHITECTURE.md](ARCHITECTURE.md) maps the app's crates and what replaces each Apple framework.

## Related

- [Jutsu Studio for Mac](https://github.com/jutsuhq/jutsu-studio): the native macOS app this one follows
- [Kawarimi](https://github.com/jutsuhq/kawarimi-local-character-swap): free local AI character swap for Windows and
  Linux, no cloud
- [awesome-higgsfield](https://github.com/jutsuhq/awesome-higgsfield): tools, SDKs, agent skills and apps for
  Higgsfield
- [More from Jutsu Studio](https://jutsustudio.com/)

## Contributing

Issues and PRs are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) and the [code of conduct](CODE_OF_CONDUCT.md) first, and keep the app's principles:
bring-your-own-account, local-first, never surprise with cost.

## Disclaimer

Jutsu Studio is an **unofficial, community-built tool**. It is not affiliated with, endorsed by, or connected to
Higgsfield AI in any way. Higgsfield, Genjutsu, Seedance, Kling and other model names are trademarks of their
respective owners. All generation costs are billed to your own Higgsfield account at list prices.

## Credits

The installers include or download FFmpeg, Real-ESRGAN ncnn Vulkan, whisper.cpp and the Whisper Small model, and the
app uses the Barlow Condensed font. [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) lists each with its license.

## License

[MIT](LICENSE) © Jutsu Studio contributors
