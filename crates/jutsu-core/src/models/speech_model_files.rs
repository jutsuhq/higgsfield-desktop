//! The files of a model the app downloads itself, pinned to one revision.

use std::fmt::Write;

use url::Url;

/// One file of a model.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ManifestFile {
    /// Relative to the repository and to the model's folder.
    pub path: &'static str,
    /// Its size in bytes.
    pub size: i64,
    /// SHA-256 of a file stored in Git LFS.
    pub sha256: Option<&'static str>,
    /// Git's blob hash of a small file stored in Git itself: SHA-1 of "blob <size>\0" and the content.
    pub git_blob: Option<&'static str>,
}

impl ManifestFile {
    const fn lfs(path: &'static str, size: i64, sha256: &'static str) -> Self {
        Self {
            path,
            size,
            sha256: Some(sha256),
            git_blob: None,
        }
    }

    const fn git(path: &'static str, size: i64, git_blob: &'static str) -> Self {
        Self {
            path,
            size,
            sha256: None,
            git_blob: Some(git_blob),
        }
    }
}

/// The files of a model the app downloads itself, pinned to one revision of its repository, with what each file's
/// content hashes to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelManifest {
    /// "owner/name" on Hugging Face.
    pub repository: &'static str,
    /// The full commit hash every file is downloaded at.
    pub revision: &'static str,
    /// The files.
    pub files: Vec<ManifestFile>,
}

impl ModelManifest {
    /// Every file's size together.
    pub fn size(&self) -> i64 {
        self.files.iter().map(|file| file.size).sum()
    }

    /// Where a file is downloaded from: the pinned revision, never the newest.
    pub fn url(&self, file: &ManifestFile) -> Option<Url> {
        let path: Vec<String> = file
            .path
            .split('/')
            .filter(|part| !part.is_empty())
            .map(path_escaped)
            .collect();
        Url::parse(&format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repository,
            self.revision,
            path.join("/")
        ))
        .ok()
    }
}

/// `component` percent-encoded with Foundation's `urlPathAllowed` set.
fn path_escaped(component: &str) -> String {
    let mut out = String::with_capacity(component.len());
    for byte in component.bytes() {
        let allowed = byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&byte);
        if allowed {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Parakeet Ultra: NVIDIA's Parakeet TDT 0.6B v3 post-trained by moondream, as a Core ML build by FluidInference,
/// run through FluidAudio on macOS. The app downloads it itself, at a pinned revision, only when the person asks in
/// Settings. Only the manifest is ported; running it needs Core ML.
pub struct ParakeetUltra;

impl ParakeetUltra {
    /// The Hugging Face repository.
    pub const REPOSITORY: &'static str = "FluidInference/parakeet-ultra-coreml";
    /// The pinned commit.
    pub const REVISION: &'static str = "95eaa59a39d4394f047a4dc5cce480388a60d1b6";
    /// About what it takes in memory while it runs; a run is refused with less free.
    pub const MEMORY_MB: i64 = 1500;
    /// Its license.
    pub const LICENSE: &'static str = "CC BY 4.0";
    /// The license's text.
    pub const LICENSE_URL: &'static str = "https://creativecommons.org/licenses/by/4.0/";
    /// The languages it hears, by code.
    pub const LANGUAGES: [&'static str; 25] = [
        "bg", "cs", "da", "de", "el", "en", "es", "et", "fi", "fr", "hr", "hu", "it", "lt", "lv", "mt", "nl",
        "pl", "pt", "ro", "ru", "sk", "sl", "sv", "uk",
    ];

    /// The model's page at the pinned revision.
    pub fn model_url() -> String {
        format!(
            "https://huggingface.co/{}/tree/{}",
            Self::REPOSITORY,
            Self::REVISION
        )
    }

    /// Its files.
    pub fn manifest() -> ModelManifest {
        ModelManifest {
            repository: Self::REPOSITORY,
            revision: Self::REVISION,
            files: vec![
                ManifestFile::lfs(
                    "Decoder.mlmodelc/analytics/coremldata.bin",
                    243,
                    "fe92b6cfaa012abd5248c0bc877832f19807015abffc60d87b8ccc8ccb48b3b5",
                ),
                ManifestFile::lfs(
                    "Decoder.mlmodelc/coremldata.bin",
                    560,
                    "3b06e66768f0df7e21795f50e2b29300e33eeb1a2579dc42c695279c2d308497",
                ),
                ManifestFile::git(
                    "Decoder.mlmodelc/model.mil",
                    13110,
                    "36dcd5cbb71d13f42a177427ddbab7539e0d167e",
                ),
                ManifestFile::lfs(
                    "Decoder.mlmodelc/weights/weight.bin",
                    23_604_992,
                    "02a0d219f281b9665bc10c8768649403b2eebbbf4b44c627041d948e0de11bb4",
                ),
                ManifestFile::lfs(
                    "Encoder.mlmodelc/analytics/coremldata.bin",
                    243,
                    "d87101d824d6723cf95304da33755c3c60e762663b9ef2b0c4bd0aa166a09a0d",
                ),
                ManifestFile::lfs(
                    "Encoder.mlmodelc/coremldata.bin",
                    514,
                    "397a84a4062f563cbc5f56077c674f09a61d85be5090f61d2f1932afb92ac0fe",
                ),
                ManifestFile::git(
                    "Encoder.mlmodelc/model.mil",
                    1_002_653,
                    "8c5c9915fc71bb7e9b0be1e7b55e38ec6a007e7f",
                ),
                ManifestFile::lfs(
                    "Encoder.mlmodelc/weights/weight.bin",
                    594_211_328,
                    "315ba01f33cadbf601d43ac7f5c86208b7aa75fdaa34705c9869d5abe3521c9b",
                ),
                ManifestFile::lfs(
                    "JointDecisionv3.mlmodelc/analytics/coremldata.bin",
                    243,
                    "68d38ca646aebafa7a9329e2efda50f5767c49e89fdb5f77f212072bb66f97c4",
                ),
                ManifestFile::lfs(
                    "JointDecisionv3.mlmodelc/coremldata.bin",
                    592,
                    "5e3af5a4ce686f6c237cadbd9284e10d333bc0e1546633cd0e430e6194044bc4",
                ),
                ManifestFile::git(
                    "JointDecisionv3.mlmodelc/model.mil",
                    11777,
                    "be552d46f98f9dc9b242b0d1659833db60aa3f83",
                ),
                ManifestFile::lfs(
                    "JointDecisionv3.mlmodelc/weights/weight.bin",
                    12_642_764,
                    "3f310b85b82341c53ec383025ab094a4e462ee1c592e1ad7c6bfe39cff66ca25",
                ),
                ManifestFile::lfs(
                    "Preprocessor.mlmodelc/analytics/coremldata.bin",
                    243,
                    "c9beeb989c8d66f8be11df59bc6df277ec76cee404f6865b46243835ef562f6d",
                ),
                ManifestFile::lfs(
                    "Preprocessor.mlmodelc/coremldata.bin",
                    486,
                    "dbde3f2300842c1fd51ef3ff948a0bcffe65ffd2dca10707f2509f32c1d65b1d",
                ),
                ManifestFile::git(
                    "Preprocessor.mlmodelc/metadata.json",
                    2841,
                    "06162c58cf2f5343a0e60f36f843bcdf3cc58527",
                ),
                ManifestFile::git(
                    "Preprocessor.mlmodelc/model.mil",
                    28181,
                    "f987e2479228436c49189aa47edb376db74dcff5",
                ),
                ManifestFile::lfs(
                    "Preprocessor.mlmodelc/weights/weight.bin",
                    491_072,
                    "129b76e3aeafa8afa3ea76d995b964b145fe83700d579f6ff42c4c38fa0968ea",
                ),
                ManifestFile::git(
                    "parakeet_vocab.json",
                    151_122,
                    "c684822ec3b671ebc449c433a7012398d39b8bb0",
                ),
            ],
        }
    }
}

/// The speech model whisper.cpp hears with on Windows and Linux: OpenAI's Whisper small, multilingual, in the ggml
/// build the whisper.cpp project publishes, quantized to 5 bits (`q5_1`). It is the built-in engine's model here, as
/// macOS's own models are on a Mac: about the size of one of those (≈ 200 MB), downloaded once after the person
/// agreed to it in the Subtitles sheet, or from Settings › Subtitles, and checked by SHA-256.
pub struct WhisperModel;

impl WhisperModel {
    /// The Hugging Face repository.
    pub const REPOSITORY: &'static str = "ggerganov/whisper.cpp";
    /// The pinned commit.
    pub const REVISION: &'static str = "5359861c739e955e79d9a303bcbc70fb988958b1";
    /// The model file, in the repository and in the model's folder.
    pub const FILE: &'static str = "ggml-small-q5_1.bin";
    /// What the model is called in Settings.
    pub const NAME: &'static str = "Whisper Small";
    /// Its license.
    pub const LICENSE: &'static str = "MIT";
    /// The license's text.
    pub const LICENSE_URL: &'static str = "https://github.com/openai/whisper/blob/main/LICENSE";
    /// The languages whisper hears, by its codes, in whisper.cpp's order.
    pub const LANGUAGES: [&'static str; 100] = [
        "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id",
        "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg",
        "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br",
        "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so",
        "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt",
        "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su", "yue",
    ];

    /// The model's page at the pinned revision.
    pub fn model_url() -> String {
        format!(
            "https://huggingface.co/{}/blob/{}/{}",
            Self::REPOSITORY,
            Self::REVISION,
            Self::FILE
        )
    }

    /// Its file.
    pub fn manifest() -> ModelManifest {
        ModelManifest {
            repository: Self::REPOSITORY,
            revision: Self::REVISION,
            files: vec![ManifestFile::lfs(
                Self::FILE,
                190_085_487,
                "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
            )],
        }
    }

    /// The revision a transcript is cached under: another file hears other words.
    pub fn revision() -> String {
        format!("{}-{}", Self::FILE, &Self::REVISION[..8])
    }

    /// The English name of a language whisper hears, by its code, as whisper.cpp names it; the code itself for one
    /// it doesn't.
    #[allow(clippy::too_many_lines, reason = "one arm per language")]
    pub fn language_name(code: &str) -> &str {
        match code {
            "en" => "English",
            "zh" => "Chinese",
            "de" => "German",
            "es" => "Spanish",
            "ru" => "Russian",
            "ko" => "Korean",
            "fr" => "French",
            "ja" => "Japanese",
            "pt" => "Portuguese",
            "tr" => "Turkish",
            "pl" => "Polish",
            "ca" => "Catalan",
            "nl" => "Dutch",
            "ar" => "Arabic",
            "sv" => "Swedish",
            "it" => "Italian",
            "id" => "Indonesian",
            "hi" => "Hindi",
            "fi" => "Finnish",
            "vi" => "Vietnamese",
            "he" => "Hebrew",
            "uk" => "Ukrainian",
            "el" => "Greek",
            "ms" => "Malay",
            "cs" => "Czech",
            "ro" => "Romanian",
            "da" => "Danish",
            "hu" => "Hungarian",
            "ta" => "Tamil",
            "no" => "Norwegian",
            "th" => "Thai",
            "ur" => "Urdu",
            "hr" => "Croatian",
            "bg" => "Bulgarian",
            "lt" => "Lithuanian",
            "la" => "Latin",
            "mi" => "Maori",
            "ml" => "Malayalam",
            "cy" => "Welsh",
            "sk" => "Slovak",
            "te" => "Telugu",
            "fa" => "Persian",
            "lv" => "Latvian",
            "bn" => "Bengali",
            "sr" => "Serbian",
            "az" => "Azerbaijani",
            "sl" => "Slovenian",
            "kn" => "Kannada",
            "et" => "Estonian",
            "mk" => "Macedonian",
            "br" => "Breton",
            "eu" => "Basque",
            "is" => "Icelandic",
            "hy" => "Armenian",
            "ne" => "Nepali",
            "mn" => "Mongolian",
            "bs" => "Bosnian",
            "kk" => "Kazakh",
            "sq" => "Albanian",
            "sw" => "Swahili",
            "gl" => "Galician",
            "mr" => "Marathi",
            "pa" => "Punjabi",
            "si" => "Sinhala",
            "km" => "Khmer",
            "sn" => "Shona",
            "yo" => "Yoruba",
            "so" => "Somali",
            "af" => "Afrikaans",
            "oc" => "Occitan",
            "ka" => "Georgian",
            "be" => "Belarusian",
            "tg" => "Tajik",
            "sd" => "Sindhi",
            "gu" => "Gujarati",
            "am" => "Amharic",
            "yi" => "Yiddish",
            "lo" => "Lao",
            "uz" => "Uzbek",
            "fo" => "Faroese",
            "ht" => "Haitian Creole",
            "ps" => "Pashto",
            "tk" => "Turkmen",
            "nn" => "Nynorsk",
            "mt" => "Maltese",
            "sa" => "Sanskrit",
            "lb" => "Luxembourgish",
            "my" => "Myanmar",
            "bo" => "Tibetan",
            "tl" => "Tagalog",
            "mg" => "Malagasy",
            "as" => "Assamese",
            "tt" => "Tatar",
            "haw" => "Hawaiian",
            "ln" => "Lingala",
            "ha" => "Hausa",
            "ba" => "Bashkir",
            "jw" => "Javanese",
            "su" => "Sundanese",
            "yue" => "Cantonese",
            other => other,
        }
    }
}
