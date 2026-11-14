//! Requests an agent could send before several inputs and elements existed keep their exact fingerprints, so one sent
//! again after an update is the same request and is never paid twice. The values are what the macOS app's release
//! gave for the same arguments; the files don't exist, so no size goes in.

use super::command_in;
use crate::Tool;
use jutsu_agent::AgentCommand;
use serde_json::json;
use std::path::Path;

const FOLDER: &str = "/nonexistent-golden";

#[cfg(unix)]
#[test]
fn a_one_input_create_image_keeps_its_fingerprint() {
    let arguments = json!({
        "request_id": "golden-1", "model": "gpt_image_2", "prompt": "the fox in a red coat",
        "aspect_ratio": "16:9", "count": 2, "input": "/nonexistent-golden/fox.png",
        "max_cost": { "amount": 2.509, "unit": "usd" },
    });
    let Ok(AgentCommand::CreateImage(command)) = command_in(Tool::CreateImage, &arguments, Path::new(FOLDER))
    else {
        panic!("no create_image");
    };
    assert_eq!(
        command.fingerprint,
        "4bad0d4e5e60c968c6ebd36d1e74c3057c380373a78ce9ddc2d367ea437af102"
    );
}

#[cfg(unix)]
#[test]
fn a_create_video_without_elements_keeps_its_fingerprint() {
    let arguments = json!({
        "request_id": "golden-2", "mode": "motion_transfer", "model": "seedance25", "prompt": "she dances",
        "source": { "path": "/nonexistent-golden/clip.mov", "start": 1, "end": 6 },
        "images": ["/nonexistent-golden/face.png"], "resolution": "720p", "draft": true, "audio": "original",
        "max_cost": { "amount": 2.509, "unit": "usd" },
    });
    let Ok(AgentCommand::CreateVideo(command)) = command_in(Tool::CreateVideo, &arguments, Path::new(FOLDER))
    else {
        panic!("no create_video");
    };
    assert_eq!(
        command.fingerprint,
        "a0c4a0ea7b6a3c48a863ff962c5843c6dd3e7f34515b93b4e5859e993bdfd7d2"
    );
}
