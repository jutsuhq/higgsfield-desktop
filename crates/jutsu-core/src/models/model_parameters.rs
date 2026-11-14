//! Model parameters a connection may or may not send.

use crate::support::string_enum;

string_enum! {
    /// Seedance 2.5's `bitrate_mode`: how much detail the encoded result keeps. The REST schemas default to `high`,
    /// the account's `generate_video` to `standard`, and the public price doesn't depend on it.
    pub enum SeedanceBitrate {
        /// Standard.
        Standard = "standard",
        /// High.
        High = "high",
    }
}

string_enum! {
    /// Kling 3.0 Motion Control's `character_orientation`: whether the character faces the way it does in the
    /// reference image or follows the source video (the server's default).
    pub enum CharacterOrientation {
        /// As in the image.
        Image = "image",
        /// As in the video.
        Video = "video",
    }
}

string_enum! {
    /// Model parameters a connection may or may not be able to send; `Capabilities::parameters` lists the ones it
    /// can.
    pub enum ModelParameter {
        /// Seedance's `bitrate_mode`.
        SeedanceBitrate = "seedanceBitrate",
        /// Seedance's start and end frames (`image-to-video`).
        SeedanceFrames = "seedanceFrames",
        /// Kling keeps the source video's sound on the server (`keep_original_sound`). REST only: the account's
        /// model has no such parameter.
        KlingOriginalSound = "klingOriginalSound",
        /// Kling's `character_orientation`. REST only, like `KlingOriginalSound`.
        KlingOrientation = "klingOrientation",
        /// Cinema Studio 4.0's creative controls (`CinemaControls`). REST only: the account's model takes them as
        /// IDs its catalog doesn't list.
        CinemaControls = "cinemaControls",
        /// Kling keeps the source video's background (`JobSpec::keeps_video_background`, the account's
        /// `background_source`). Account only: the REST API has no background parameter.
        KlingVideoBackground = "klingVideoBackground",
    }
}
