# Third-party notices

Jutsu Studio's installers for Windows and Linux include, or download on request, the programs below. Each keeps its
own license; none of them is changed.

## FFmpeg

The installers include `ffmpeg` and `ffprobe` from FFmpeg 8.1 (`n8.1.3-34-ga35157abb9`), as built by BtbN's
FFmpeg-Builds in its LGPL configuration (no GPL or non-free parts), with libopenh264 and libass. Jutsu Studio runs them
as separate programs and doesn't link to them.

- License: GNU Lesser General Public License, version 2.1 or later — https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html
- FFmpeg's source: https://ffmpeg.org/download.html, release branch `release/8.1`, tag `n8.1.3`, and
  https://git.ffmpeg.org/ffmpeg.git
- The build scripts and the exact source revisions of the build and its libraries:
  https://github.com/BtbN/FFmpeg-Builds (release `autobuild-2026-11-09-12-42`)
- `scripts/fetch-ffmpeg.mjs` names the archives and their SHA-256 checksums.

You may replace the bundled `ffmpeg` and `ffprobe` with your own builds: set the `FFMPEG` and `FFPROBE` environment
variables to their paths, or replace the files next to the Jutsu Studio executable.

## Real-ESRGAN ncnn Vulkan

Upscale on this computer downloads Real-ESRGAN ncnn Vulkan (`realesrgan-ncnn-vulkan`, release v0.2.5.0) and its
`realesrgan-x4plus` model from the project's GitHub release, once the person agrees to the download. It is kept in the
app's data folder under `Engines`.

- License: BSD 3-Clause — https://github.com/xinntao/Real-ESRGAN/blob/master/LICENSE
- Source: https://github.com/xinntao/Real-ESRGAN and https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan

## whisper.cpp and the Whisper Small model

Subtitles made on this computer run whisper.cpp (through the `whisper-rs` crate, version 0.16.0), which is built from
source into Jutsu Studio. Its `ggml-small-q5_1.bin` build of OpenAI's Whisper Small model is downloaded from the
whisper.cpp project's Hugging Face repository once the person agrees to the download, and kept in the app's data
folder.

- whisper.cpp: MIT License — https://github.com/ggml-org/whisper.cpp/blob/master/LICENSE
- Whisper and its model weights: MIT License — https://github.com/openai/whisper/blob/main/LICENSE
- The model's source: https://huggingface.co/ggerganov/whisper.cpp

## Barlow Condensed

The app's display font is Barlow Condensed ExtraBold by The Barlow Project Authors, used unmodified.

- License: SIL Open Font License 1.1 — [src/design/fonts/OFL.txt](src/design/fonts/OFL.txt)
- Source: https://github.com/jpt/barlow
