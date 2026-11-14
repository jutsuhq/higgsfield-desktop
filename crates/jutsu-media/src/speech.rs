//! Speech heard on this computer (`OnDeviceSpeech.swift`): the sound is decoded with ffmpeg to 16 kHz mono and heard by
//! whisper.cpp with `WhisperModel`, which is downloaded once, after the person agreed to it. On macOS the built-in
//! engines are SpeechTranscriber and Dictation and Parakeet Ultra runs with Core ML; here whisper takes their place.
//!
//! A file with no sound louder than `SoundCheck::THRESHOLD` is refused as having no speech before any model is
//! loaded or downloaded. What a model heard is cached by the file's content, the language, the model and its
//! revision, so a run made again on the same file reads it back.
//!
//! whisper.cpp is built only with the `whisper` feature; without it, no language is offered.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use jutsu_core::{
    OnDeviceProgress, PortError, Progress, SpeechAvailability, SpeechModel, SpeechSelection,
    SpeechTranscribing, TimedWord, Transcript, TranscriptionError, WhisperModel,
};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::hash::{sha256_hex, sha256_hex_of_file};
use crate::probe::probe_streams;
use crate::tools::run_capture;
use crate::{FfmpegTools, ModelDownloader};

/// Whether a file's sound has anything to hear.
pub struct SoundCheck;

impl SoundCheck {
    /// Samples at or under this level, of 1, are silence.
    pub const THRESHOLD: f32 = 0.001;

    pub fn has_sound(samples: &[f32]) -> bool {
        samples.iter().any(|sample| sample.abs() > Self::THRESHOLD)
    }
}

/// The transcripts heard so far, one JSON file each, the oldest pruned past `capacity`.
#[derive(Debug, Clone)]
pub struct TranscriptCache {
    folder: PathBuf,
    capacity: usize,
}

impl TranscriptCache {
    pub const CAPACITY: usize = 200;

    pub fn new(folder: PathBuf, capacity: usize) -> Self {
        Self { folder, capacity }
    }

    fn file(&self, key: &str) -> PathBuf {
        self.folder.join(format!("{}.json", sha256_hex(key)))
    }

    pub fn read(&self, key: &str) -> Option<Transcript> {
        let data = std::fs::read(self.file(key)).ok()?;
        serde_json::from_slice(&data).ok()
    }

    /// Keeps `transcript`; a cache that can't be written only costs the next run its time.
    pub fn write(&self, key: &str, transcript: &Transcript) {
        let written = std::fs::create_dir_all(&self.folder).and_then(|()| {
            let data = serde_json::to_vec(transcript).map_err(std::io::Error::other)?;
            std::fs::write(self.file(key), data)
        });
        if let Err(error) = written {
            tracing::warn!(%error, "can't cache a transcript");
            return;
        }
        self.prune();
    }

    fn prune(&self) {
        let Ok(entries) = std::fs::read_dir(&self.folder) else {
            return;
        };
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            })
            .map(|entry| {
                let modified = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                (modified, entry.path())
            })
            .collect();
        if files.len() <= self.capacity {
            return;
        }
        files.sort();
        let excess = files.len() - self.capacity;
        for (_, path) in files.into_iter().take(excess) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// whisper.cpp on this computer, with its model in `folder`.
pub struct OnDeviceSpeech {
    tools: Option<FfmpegTools>,
    folder: PathBuf,
    downloader: ModelDownloader,
    cache: Option<TranscriptCache>,
}

impl std::fmt::Debug for OnDeviceSpeech {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OnDeviceSpeech")
            .field("folder", &self.folder)
            .finish_non_exhaustive()
    }
}

impl OnDeviceSpeech {
    /// Whether this build hears speech at all: whisper.cpp comes with the `whisper` feature.
    pub const HAS_RECOGNIZER: bool = cfg!(feature = "whisper");

    /// `folder` is the model's folder (`SpeechModels::folder_in`); `cache` the transcripts' folder, `None` for none.
    pub fn new(
        tools: Option<FfmpegTools>,
        folder: PathBuf,
        downloader: ModelDownloader,
        cache: Option<PathBuf>,
    ) -> Self {
        Self {
            tools,
            folder,
            downloader,
            cache: cache.map(|folder| TranscriptCache::new(folder, TranscriptCache::CAPACITY)),
        }
    }

    fn model_file(&self) -> PathBuf {
        self.folder.join(WhisperModel::FILE)
    }

    fn is_model_here(&self) -> bool {
        ModelDownloader::is_complete(&WhisperModel::manifest(), &self.folder)
    }
}

fn transcription(error: TranscriptionError) -> PortError {
    PortError::Transcription(error)
}

/// The file's sound as 16 kHz mono samples; a file with no sound has no speech, one ffmpeg can't read can't be opened.
pub async fn decode(tools: &FfmpegTools, file: &Path) -> Result<Vec<f32>, PortError> {
    let probe = probe_streams(tools, file.as_os_str())
        .await
        .map_err(|_| transcription(TranscriptionError::CantOpenFile))?;
    if probe.audio.is_none() {
        return Err(transcription(TranscriptionError::NoSpeech));
    }
    let mut args: Vec<OsString> = ["-nostdin", "-v", "error", "-i"].map(OsString::from).to_vec();
    args.push(file.as_os_str().to_owned());
    args.extend(["-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-"].map(OsString::from));
    let bytes = run_capture(&tools.ffmpeg, args, &CancellationToken::new())
        .await
        .map_err(|_| transcription(TranscriptionError::CantOpenFile))?;
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect())
}

#[async_trait]
impl SpeechTranscribing for OnDeviceSpeech {
    async fn availability(&self) -> SpeechAvailability {
        let mut availability = SpeechAvailability::default();
        if !Self::HAS_RECOGNIZER || self.tools.is_none() {
            return availability;
        }
        let languages: Vec<String> = WhisperModel::LANGUAGES
            .iter()
            .map(|&code| code.to_owned())
            .collect();
        if self.is_model_here() {
            availability
                .installed
                .insert(SpeechModel::Whisper, languages.iter().cloned().collect());
        }
        availability.languages.insert(SpeechModel::Whisper, languages);
        availability
    }

    async fn transcribe(
        &self,
        file: &Url,
        language: &str,
        model: SpeechModel,
        may_download: bool,
        progress: Progress<OnDeviceProgress>,
    ) -> Result<Transcript, PortError> {
        let code = SpeechSelection::language_code(language);
        if model != SpeechModel::Whisper || !WhisperModel::LANGUAGES.contains(&code.as_str()) {
            return Err(transcription(TranscriptionError::UnsupportedLanguage));
        }
        let path = file
            .to_file_path()
            .ok()
            .filter(|path| path.is_file())
            .ok_or_else(|| transcription(TranscriptionError::CantOpenFile))?;
        let Some(tools) = &self.tools else {
            return Err(PortError::Other(
                "ffmpeg isn't installed, so the sound can't be read.".to_owned(),
            ));
        };
        let revision = WhisperModel::revision();
        let hashed = path.clone();
        let content = tokio::task::spawn_blocking(move || sha256_hex_of_file(&hashed))
            .await
            .map_err(|error| PortError::Other(error.to_string()))?
            .map_err(|_| transcription(TranscriptionError::CantOpenFile))?;
        let key = SpeechSelection::cache_key(&content, language, model, &revision);
        if let Some(cached) = self.cache.as_ref().and_then(|cache| cache.read(&key)) {
            return Ok(cached);
        }
        let samples = decode(tools, &path).await?;
        if !SoundCheck::has_sound(&samples) {
            return Err(transcription(TranscriptionError::NoSpeech));
        }
        if !self.is_model_here() {
            if !may_download {
                return Err(transcription(TranscriptionError::ModelNeeded));
            }
            let downloading = Arc::clone(&progress);
            let report: Progress<f64> =
                Arc::new(move |fraction| downloading(OnDeviceProgress::DownloadingModel(fraction)));
            SpeechModelDownloader::download(&self.downloader, &self.folder, report).await?;
        }
        let words = recognize(self.model_file(), code, samples, progress).await?;
        if words.is_empty() {
            return Err(transcription(TranscriptionError::NoSpeech));
        }
        let transcript = Transcript {
            language: language.to_owned(),
            model,
            revision,
            words,
        };
        if let Some(cache) = &self.cache {
            cache.write(&key, &transcript);
        }
        Ok(transcript)
    }
}

/// The model's download through the port, cancelled with the run.
struct SpeechModelDownloader;

impl SpeechModelDownloader {
    async fn download(
        downloader: &ModelDownloader,
        folder: &Path,
        progress: Progress<f64>,
    ) -> Result<(), PortError> {
        Ok(downloader
            .download(
                &WhisperModel::manifest(),
                folder,
                Some(progress),
                &CancellationToken::new(),
            )
            .await?)
    }
}

/// Sets the flag when the run's future is dropped, so whisper stops at its next check.
#[cfg(feature = "whisper")]
struct AbortOnDrop(Arc<std::sync::atomic::AtomicBool>);

#[cfg(feature = "whisper")]
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(feature = "whisper")]
async fn recognize(
    model: PathBuf,
    language: String,
    samples: Vec<f32>,
    progress: Progress<OnDeviceProgress>,
) -> Result<Vec<TimedWord>, PortError> {
    let abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let guard = AbortOnDrop(Arc::clone(&abort));
    let heard =
        tokio::task::spawn_blocking(move || whisper::hear(&model, &language, &samples, progress, &abort))
            .await
            .map_err(|error| PortError::Other(error.to_string()))?;
    drop(guard);
    heard
}

#[cfg(not(feature = "whisper"))]
#[allow(clippy::unused_async, reason = "the same signature as whisper's")]
async fn recognize(
    _model: PathBuf,
    _language: String,
    _samples: Vec<f32>,
    _progress: Progress<OnDeviceProgress>,
) -> Result<Vec<TimedWord>, PortError> {
    Err(transcription(TranscriptionError::UnsupportedLanguage))
}

#[cfg(feature = "whisper")]
mod whisper {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use jutsu_core::{OnDeviceProgress, PortError, Progress, TimedWord, WordSpacing};
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    fn failed(error: &whisper_rs::WhisperError) -> PortError {
        PortError::Other(format!("The speech model failed: {error}"))
    }

    /// whisper's times are in hundredths of a second.
    #[allow(clippy::cast_precision_loss, reason = "hundredths of a second of a video")]
    fn seconds(centiseconds: i64) -> f64 {
        centiseconds.max(0) as f64 / 100.0
    }

    /// Hears `samples` in `language` and gives its words with their times: each token is a run, which
    /// `WordSpacing::append` joins into words; a token ending in the middle of a character waits for the next.
    pub(super) fn hear(
        model: &Path,
        language: &str,
        samples: &[f32],
        progress: Progress<OnDeviceProgress>,
        abort: &Arc<AtomicBool>,
    ) -> Result<Vec<TimedWord>, PortError> {
        let path = model
            .to_str()
            .ok_or_else(|| PortError::Other("The speech model's path can't be read.".to_owned()))?;
        let context = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|error| failed(&error))?;
        let mut state = context.create_state().map_err(|error| failed(&error))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language));
        params.set_token_timestamps(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        let threads = std::thread::available_parallelism().map_or(2, |count| count.get().min(4));
        params.set_n_threads(i32::try_from(threads).unwrap_or(2));
        params.set_progress_callback_safe(move |percent: i32| {
            progress(OnDeviceProgress::Working(
                f64::from(percent.clamp(0, 100)) / 100.0,
            ));
        });
        let stop = Arc::clone(abort);
        // whisper-rs 0.16's trampoline reads the closure as its own type although it is stored boxed as
        // `Box<dyn FnMut() -> bool>`; handing it that boxed type makes the two the same.
        let abort_callback: Box<dyn FnMut() -> bool> = Box::new(move || stop.load(Ordering::SeqCst));
        params.set_abort_callback_safe(abort_callback);
        let heard = state.full(params, samples);
        if abort.load(Ordering::SeqCst) {
            return Err(PortError::Cancelled);
        }
        heard.map_err(|error| failed(&error))?;
        let end_of_text = context.token_eot();
        let mut words = Vec::new();
        for segment in state.as_iter() {
            let mut runs = Vec::new();
            let mut pending: Vec<u8> = Vec::new();
            let mut start = 0;
            for index in 0..segment.n_tokens() {
                let Some(token) = segment.get_token(index) else {
                    continue;
                };
                // The special tokens, timestamps among them, come after the text's.
                if token.token_id() >= end_of_text {
                    continue;
                }
                let Ok(bytes) = token.to_bytes() else {
                    continue;
                };
                let data = token.token_data();
                if pending.is_empty() {
                    start = data.t0;
                }
                pending.extend_from_slice(bytes);
                if let Ok(text) = std::str::from_utf8(&pending) {
                    runs.push(TimedWord::new(text, seconds(start), seconds(data.t1)));
                    pending.clear();
                }
            }
            WordSpacing::append(&runs, &mut words, language);
        }
        Ok(words)
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::test_support::{dir, tools};

    fn clip(folder: &Path, name: &str, source: &str) -> PathBuf {
        let path = folder.join(name);
        let status = Command::new(&tools().expect("ffmpeg").ffmpeg)
            .args(["-v", "error", "-f", "lavfi", "-i", source, "-t", "1", "-ac", "1"])
            .arg(&path)
            .status()
            .expect("ffmpeg runs");
        assert!(status.success());
        path
    }

    fn speech(folder: &Path) -> OnDeviceSpeech {
        OnDeviceSpeech::new(
            tools(),
            folder.join("model"),
            ModelDownloader::new(reqwest::Client::new()),
            Some(folder.join("transcripts")),
        )
    }

    fn progress() -> Progress<OnDeviceProgress> {
        Arc::new(|_| {})
    }

    async fn heard(speech: &OnDeviceSpeech, file: &Path, language: &str) -> Result<Transcript, PortError> {
        let url = Url::from_file_path(file).expect("a file URL");
        speech
            .transcribe(&url, language, SpeechModel::Whisper, false, progress())
            .await
    }

    #[test]
    fn silence_is_at_or_under_the_threshold() {
        assert!(!SoundCheck::has_sound(&[0.0, 0.001, -0.001]));
        assert!(SoundCheck::has_sound(&[0.0, -0.002]));
    }

    #[tokio::test]
    async fn a_silent_file_has_no_speech_and_needs_no_model() {
        if tools().is_none() {
            return;
        }
        let folder = dir();
        let silent = clip(folder.path(), "silent.wav", "anullsrc=r=16000:cl=mono");
        let speech = speech(folder.path());
        assert_eq!(
            heard(&speech, &silent, "en").await,
            Err(PortError::Transcription(TranscriptionError::NoSpeech))
        );
    }

    #[tokio::test]
    async fn a_sound_waits_for_the_model_the_person_agreed_to() {
        if tools().is_none() {
            return;
        }
        let folder = dir();
        let tone = clip(folder.path(), "tone.wav", "sine=frequency=440:sample_rate=16000");
        let speech = speech(folder.path());
        assert_eq!(
            heard(&speech, &tone, "en-US").await,
            Err(PortError::Transcription(TranscriptionError::ModelNeeded))
        );
        assert_eq!(
            heard(&speech, &folder.path().join("missing.wav"), "en").await,
            Err(PortError::Transcription(TranscriptionError::CantOpenFile))
        );
        assert_eq!(
            heard(&speech, &tone, "tlh").await,
            Err(PortError::Transcription(TranscriptionError::UnsupportedLanguage))
        );
    }

    #[tokio::test]
    async fn the_languages_come_with_the_recognizer() {
        let folder = dir();
        let availability = speech(folder.path()).availability().await;
        let offered = availability.languages.get(&SpeechModel::Whisper).map(Vec::len);
        let expected = (OnDeviceSpeech::HAS_RECOGNIZER && tools().is_some()).then_some(100);
        assert_eq!(offered, expected);
        assert!(availability.installed.is_empty());
    }

    #[tokio::test]
    async fn a_heard_file_is_read_back_from_the_cache() {
        if tools().is_none() {
            return;
        }
        let folder = dir();
        let tone = clip(folder.path(), "tone.wav", "sine=frequency=440:sample_rate=16000");
        let speech = speech(folder.path());
        let content = sha256_hex_of_file(&tone).expect("hashed");
        let key = SpeechSelection::cache_key(&content, "en", SpeechModel::Whisper, &WhisperModel::revision());
        let transcript = Transcript {
            language: "en".to_owned(),
            model: SpeechModel::Whisper,
            revision: WhisperModel::revision(),
            words: vec![TimedWord::new("Hi", 0.0, 0.4)],
        };
        speech.cache.as_ref().expect("a cache").write(&key, &transcript);
        assert_eq!(heard(&speech, &tone, "en").await, Ok(transcript));
    }

    /// A real run: downloads the model into `JUTSU_WHISPER_FOLDER` (190 MB, checked against its SHA-256) and hears
    /// `JUTSU_SPEECH_CLIP`, a few seconds of English speech. Run with `--features whisper -- --ignored`.
    #[tokio::test]
    #[ignore = "downloads the 190 MB model and runs whisper.cpp"]
    async fn hears_real_speech() {
        let (Some(folder), Some(clip)) = (
            std::env::var_os("JUTSU_WHISPER_FOLDER"),
            std::env::var_os("JUTSU_SPEECH_CLIP"),
        ) else {
            return;
        };
        let folder = PathBuf::from(folder);
        let speech = OnDeviceSpeech::new(
            tools(),
            folder.join("model"),
            ModelDownloader::new(reqwest::Client::new()),
            None,
        );
        let fractions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&fractions);
        let progress: Progress<OnDeviceProgress> = Arc::new(move |step| {
            if let Ok(mut seen) = seen.lock() {
                seen.push(step);
            }
        });
        let url = Url::from_file_path(PathBuf::from(clip)).expect("a file URL");
        let transcript = speech
            .transcribe(&url, "en-US", SpeechModel::Whisper, true, progress)
            .await
            .expect("heard");
        assert!(!transcript.text().is_empty(), "no words heard");
        assert!(
            transcript
                .words
                .windows(2)
                .all(|pair| pair[0].start <= pair[1].start)
        );
    }

    #[test]
    fn the_cache_keeps_only_the_newest() {
        let folder = dir();
        let cache = TranscriptCache::new(folder.path().to_path_buf(), 2);
        let transcript = Transcript {
            language: "en".to_owned(),
            model: SpeechModel::Whisper,
            revision: "1".to_owned(),
            words: Vec::new(),
        };
        for key in ["a", "b", "c"] {
            cache.write(key, &transcript);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(cache.read("a").is_none());
        assert!(cache.read("b").is_some() && cache.read("c").is_some());
    }
}
