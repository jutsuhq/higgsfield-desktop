//! Real-ESRGAN ncnn-vulkan: the engine Upscale on this computer runs. Its official release archive is downloaded once,
//! after the person agrees, and checked against the SHA-256 recorded here before anything in it runs.

/// One official release archive of Real-ESRGAN ncnn-vulkan for one platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EngineArchive {
    /// The release it belongs to.
    pub tag: &'static str,
    /// The archive's address on the project's GitHub releases.
    pub url: &'static str,
    /// Its size in bytes.
    pub size: i64,
    /// SHA-256 of the archive, as computed from the downloaded file when it was pinned.
    pub sha256: &'static str,
    /// The executable's path inside the archive.
    pub executable: &'static str,
    /// The other files the engine needs, by their path inside the archive.
    pub files: &'static [&'static str],
}

/// Real-ESRGAN ncnn-vulkan (BSD-3-Clause), pinned to one release.
pub struct RealEsrgan;

impl RealEsrgan {
    /// The pinned release.
    pub const TAG: &'static str = "v0.2.5.0";
    /// The model every upscale runs: Real-ESRGAN x4plus, for photographs and real footage.
    pub const MODEL: &'static str = "realesrgan-x4plus";
    /// Its only factor.
    pub const SCALE: i64 = 4;
    /// The project's page.
    pub const PROJECT_URL: &'static str = "https://github.com/xinntao/Real-ESRGAN";
    /// Its license.
    pub const LICENSE: &'static str = "BSD-3-Clause";
    /// The folder of the app's data dir the engine is unpacked into, one subfolder per release.
    pub const FOLDER: &'static str = "Engines";

    /// The archive for Windows on x86-64.
    pub const WINDOWS_X64: EngineArchive = EngineArchive {
        tag: Self::TAG,
        url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-windows.zip",
        size: 45_474_481,
        sha256: "abc02804e17982a3be33675e4d471e91ea374e65b70167abc09e31acb412802d",
        executable: "realesrgan-ncnn-vulkan.exe",
        files: &[
            "vcomp140.dll",
            "models/realesrgan-x4plus.bin",
            "models/realesrgan-x4plus.param",
        ],
    };

    /// The archive for Linux on x86-64 (built on Ubuntu).
    pub const LINUX_X64: EngineArchive = EngineArchive {
        tag: Self::TAG,
        url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-ubuntu.zip",
        size: 46_931_474,
        sha256: "e5aa6eb131234b87c0c51f82b89390f5e3e642b7b70f2b9bbe95b6a285a40c96",
        executable: "realesrgan-ncnn-vulkan",
        files: &["models/realesrgan-x4plus.bin", "models/realesrgan-x4plus.param"],
    };

    /// The archive for macOS, which development builds run on.
    pub const MACOS: EngineArchive = EngineArchive {
        tag: Self::TAG,
        url: "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-macos.zip",
        size: 51_817_124,
        sha256: "e0ad05580abfeb25f8d8fb55aaf7bedf552c375b5b4d9bd3c8d59764d2cc333a",
        executable: "realesrgan-ncnn-vulkan",
        files: &["models/realesrgan-x4plus.bin", "models/realesrgan-x4plus.param"],
    };

    /// The archive for `os` and `arch` (`std::env::consts`' names); `None` where the project publishes none, as
    /// for Linux on ARM.
    pub fn archive(os: &str, arch: &str) -> Option<EngineArchive> {
        match (os, arch) {
            ("windows", "x86_64") => Some(Self::WINDOWS_X64),
            ("linux", "x86_64") => Some(Self::LINUX_X64),
            ("macos", "x86_64" | "aarch64") => Some(Self::MACOS),
            _ => None,
        }
    }

    /// The archive for this computer.
    pub fn current() -> Option<EngineArchive> {
        Self::archive(std::env::consts::OS, std::env::consts::ARCH)
    }

    /// Why this computer has no engine: the project publishes no build for it.
    pub fn unavailable_reason(os: &str, arch: &str) -> String {
        format!("Real-ESRGAN publishes no build for {os} on {arch}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_platform_has_its_pinned_archive() {
        for archive in [RealEsrgan::WINDOWS_X64, RealEsrgan::LINUX_X64, RealEsrgan::MACOS] {
            assert_eq!(archive.tag, RealEsrgan::TAG);
            assert!(archive.url.contains("/releases/download/v0.2.5.0/"));
            assert_eq!(archive.sha256.len(), 64);
            assert!(archive.size > 40_000_000);
            assert!(archive.files.contains(&"models/realesrgan-x4plus.bin"));
        }
        assert_eq!(
            RealEsrgan::archive("windows", "x86_64"),
            Some(RealEsrgan::WINDOWS_X64)
        );
        assert_eq!(
            RealEsrgan::archive("linux", "x86_64"),
            Some(RealEsrgan::LINUX_X64)
        );
        assert_eq!(RealEsrgan::archive("linux", "aarch64"), None);
        assert_eq!(
            RealEsrgan::unavailable_reason("linux", "aarch64"),
            "Real-ESRGAN publishes no build for linux on aarch64."
        );
    }
}
