//! The parsers of `create_image` and the Library tools, kept apart from the video tools in `tools`.

use crate::{
    arguments::{Arguments, refuse},
    inbox::Inbox,
    tools::tool_fingerprint,
};
use jutsu_agent::{
    AgentCommand, AgentCulling, AgentDeliveryAction, AgentEnhancement, AgentError, AgentExportPreset,
    AgentImageRequest, AgentLibraryKind, AgentSubtitleEngine, AgentSubtitleFormat, AgentSubtitlesAction,
    AgentVerdict, CreateElementCommand, CreateImageCommand, DeliverySetCommand, ElementsCommand,
    EnhanceCommand, ExportCommand, FramesCommand, LibraryDownloadCommand, LibraryGetCommand,
    LibraryMarkCommand, LibrarySearchCommand, SubtitlesCommand, UploadRequest, Uuid, fingerprint,
};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// The fingerprint a free or not yet fingerprinted command carries while it is checked.
fn zeros() -> String {
    "0".repeat(64)
}

fn local(path: &str, working_directory: &Path) -> String {
    Inbox::resolve(path, working_directory)
        .to_string_lossy()
        .into_owned()
}

pub(crate) fn create_image(given: &Arguments, working_directory: &Path) -> Result<AgentCommand, AgentError> {
    given.allow(&[
        "request_id",
        "model",
        "prompt",
        "aspect_ratio",
        "count",
        "inputs",
        "input",
        "elements",
        "max_cost",
        "title",
    ])?;
    let request_id = given.request_id()?;
    let max_cost = given.max_cost()?;
    let Some(prompt) = given.string("prompt")? else {
        return Err(refuse("create_image needs prompt, the image to make"));
    };
    // input is the one-image form of inputs; an agent sends one or the other.
    let input = given.string("input")?;
    let mut inputs = given.strings("inputs")?;
    if let Some(input) = input {
        if !inputs.is_empty() || given.given("inputs").is_some() {
            return Err(refuse(format!(
                "create_image: {}",
                AgentImageRequest::BOTH_INPUTS
            )));
        }
        inputs = vec![input];
    }
    let mut image = AgentImageRequest {
        model: given.string("model")?,
        prompt,
        aspect_ratio: given.string("aspect_ratio")?,
        count: given.int("count")?,
        inputs,
        elements: given.strings("elements")?,
        title: given.string("title")?,
    };
    if let Some(problem) = image.problem() {
        return Err(refuse(format!("create_image: {problem}")));
    }
    image.inputs = image
        .inputs
        .iter()
        .map(|path| local(path, working_directory))
        .collect();
    let fingerprint = tool_fingerprint("create_image", &image, &image.inputs, Some(max_cost))?;
    Ok(AgentCommand::CreateImage(CreateImageCommand {
        request_id,
        fingerprint,
        image,
        max_cost,
    }))
}

pub(crate) fn library_search(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&[
        "query",
        "kind",
        "starred",
        "verdict",
        "include_rejected",
        "model",
        "mode",
        "from",
        "to",
        "limit",
        "cursor",
        "include_higgsfield",
        "refresh_higgsfield",
    ])?;
    let kind = given
        .string("kind")?
        .map(|name| {
            AgentLibraryKind::parse(&name)
                .ok_or_else(|| refuse("library_search: kind is all, video or image"))
        })
        .transpose()?;
    let verdict = given
        .string("verdict")?
        .map(|name| {
            AgentVerdict::parse(&name)
                .ok_or_else(|| refuse("library_search: verdict is unreviewed, kept or rejected"))
        })
        .transpose()?;
    let command = LibrarySearchCommand {
        query: given.string("query")?,
        kind,
        starred: given.optional_bool("starred")?,
        verdict,
        include_rejected: given.optional_bool("include_rejected")?,
        model: given.string("model")?,
        mode: given.string("mode")?,
        from: given.date("from")?,
        to: given.date("to")?,
        limit: given.int("limit")?,
        cursor: given.string("cursor")?,
        include_higgsfield: given.optional_bool("include_higgsfield")?,
        refresh_higgsfield: given.optional_bool("refresh_higgsfield")?,
    };
    if let Some(problem) = command.problem() {
        return Err(refuse(format!("library_search: {problem}")));
    }
    Ok(AgentCommand::LibrarySearch(command))
}

pub(crate) fn library_get(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&["ids"])?;
    let command = LibraryGetCommand {
        ids: given.ids("ids")?,
    };
    if let Some(problem) = command.problem() {
        return Err(refuse(format!("library_get: {problem}")));
    }
    Ok(AgentCommand::LibraryGet(command))
}

pub(crate) fn elements(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&["query", "category", "limit", "cursor"])?;
    let command = ElementsCommand {
        query: given.string("query")?,
        category: given.string("category")?,
        limit: given.int("limit")?,
        cursor: given.string("cursor")?,
    };
    if let Some(problem) = command.problem() {
        return Err(refuse(format!("elements: {problem}")));
    }
    Ok(AgentCommand::Elements(command))
}

pub(crate) fn create_element(
    given: &Arguments,
    working_directory: &Path,
) -> Result<AgentCommand, AgentError> {
    #[derive(Serialize)]
    struct Fields<'a> {
        name: &'a str,
        category: &'a str,
        images: &'a [String],
    }
    given.allow(&["request_id", "name", "category", "images"])?;
    let request_id = given.request_id()?;
    let Some(name) = given.string("name")? else {
        return Err(refuse(
            "create_element needs name, what create_video's and create_image's elements call it",
        ));
    };
    let mut command = CreateElementCommand {
        request_id,
        fingerprint: zeros(),
        name,
        category: given.string("category")?,
        images: given.strings("images")?,
    };
    if let Some(problem) = command.problem() {
        return Err(refuse(format!("create_element: {problem}")));
    }
    command.images = command
        .images
        .iter()
        .map(|path| local(path, working_directory))
        .collect();
    let fields = Fields {
        name: &command.name,
        category: command
            .category
            .as_deref()
            .unwrap_or(CreateElementCommand::DEFAULT_CATEGORY),
        images: &command.images,
    };
    let fingerprint = tool_fingerprint("create_element", &fields, &command.images, None)?;
    command.fingerprint = fingerprint;
    Ok(AgentCommand::CreateElement(command))
}

pub(crate) fn library_download(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&["ids"])?;
    let command = LibraryDownloadCommand {
        ids: given.ids("ids")?,
    };
    if let Some(problem) = command.problem() {
        return Err(refuse(format!("library_download: {problem}")));
    }
    Ok(AgentCommand::LibraryDownload(command))
}

pub(crate) fn library_mark(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&["ids", "verdict", "star"])?;
    let verdict = given
        .string("verdict")?
        .map(|name| {
            AgentCulling::parse(&name).ok_or_else(|| refuse("library_mark: verdict is keep, reject or clear"))
        })
        .transpose()?;
    let command = LibraryMarkCommand {
        ids: given.ids("ids")?,
        verdict,
        star: given.optional_bool("star")?,
    };
    if let Some(problem) = command.problem() {
        return Err(refuse(format!("library_mark: {problem}")));
    }
    Ok(AgentCommand::LibraryMark(command))
}

pub(crate) fn enhance(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&[
        "request_id",
        "item",
        "operation",
        "target",
        "frame_rate",
        "preset",
        "engine",
        "mode",
        "aspect_ratio",
        "language",
        "voice",
        "max_cost",
    ])?;
    let request_id = given.request_id()?;
    let Some(item) = given.string("item")?.as_deref().and_then(Uuid::parse) else {
        return Err(refuse("enhance needs item, a Library ID from library_search"));
    };
    let Some(operation) = given
        .string("operation")?
        .as_deref()
        .and_then(AgentEnhancement::parse)
    else {
        return Err(refuse(format!(
            "enhance needs operation, one of {}",
            AgentEnhancement::names()
        )));
    };
    let engine = given.string("engine")?;
    // A run on this computer is free: max_cost, sent or not, plays no part in it or in its fingerprint.
    let max_cost = if EnhanceCommand::runs_on_device(operation, engine.as_deref()) {
        None
    } else {
        Some(given.max_cost()?)
    };
    let mut command = EnhanceCommand {
        request_id,
        fingerprint: zeros(),
        item,
        operation,
        target: given.string("target")?,
        frame_rate: given.int("frame_rate")?,
        preset: given.string("preset")?,
        engine,
        mode: given.string("mode")?,
        aspect_ratio: given.string("aspect_ratio")?,
        language: given.string("language")?,
        voice: given.string("voice")?,
        max_cost,
    };
    if let Some(problem) = command.problem() {
        return Err(refuse(format!("enhance: {problem}")));
    }
    command.fingerprint = tool_fingerprint("enhance", &command, &[], max_cost)?;
    Ok(AgentCommand::Enhance(command))
}

pub(crate) fn subtitles(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&["action", "item", "request_id", "language", "engine", "format"])?;
    let Some(action) = given
        .string("action")?
        .as_deref()
        .and_then(AgentSubtitlesAction::parse)
    else {
        return Err(refuse("subtitles needs action: make, get or export"));
    };
    let Some(item) = given.string("item")?.as_deref().and_then(Uuid::parse) else {
        return Err(refuse(
            "subtitles needs item, a video's Library ID from library_search",
        ));
    };
    let engine = given
        .string("engine")?
        .map(|text| {
            AgentSubtitleEngine::parse(&text).ok_or_else(|| {
                refuse(format!(
                    "subtitles: engine is one of {}",
                    AgentSubtitleEngine::names()
                ))
            })
        })
        .transpose()?;
    let format = given
        .string("format")?
        .map(|text| {
            AgentSubtitleFormat::parse(&text).ok_or_else(|| refuse("subtitles: format is srt or vtt"))
        })
        .transpose()?;
    let request_id = if action == AgentSubtitlesAction::Make {
        Some(given.request_id()?)
    } else {
        given.string("request_id")?
    };
    let mut command = SubtitlesCommand {
        action,
        item,
        request_id,
        fingerprint: None,
        language: given.string("language")?,
        engine,
        format,
    };
    if action == AgentSubtitlesAction::Make {
        command.fingerprint = Some(zeros());
        if let Some(problem) = command.problem() {
            return Err(refuse(format!("subtitles: {problem}")));
        }
        // Free: the fingerprint is the request's parameters alone.
        command.fingerprint = None;
        command.fingerprint = Some(tool_fingerprint("subtitles", &command, &[], None)?);
    }
    if let Some(problem) = command.problem() {
        return Err(refuse(format!("subtitles: {problem}")));
    }
    Ok(AgentCommand::Subtitles(command))
}

pub(crate) fn delivery_set(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&["action", "name", "items", "set", "folder", "wait_seconds"])?;
    let Some(action) = given
        .string("action")?
        .as_deref()
        .and_then(AgentDeliveryAction::parse)
    else {
        return Err(refuse("delivery_set needs action: create, list or export"));
    };
    let set = given.uuid("set", "delivery_set: set is a set ID")?;
    let command = DeliverySetCommand {
        name: given.string("name")?,
        items: given.ids("items")?,
        set,
        folder: given.string("folder")?,
        wait_seconds: given.int("wait_seconds")?,
        ..DeliverySetCommand::new(action)
    };
    if let Some(problem) = command.problem() {
        return Err(refuse(format!("delivery_set: {problem}")));
    }
    Ok(AgentCommand::DeliverySet(command))
}

pub(crate) fn frames(given: &Arguments, working_directory: &Path) -> Result<AgentCommand, AgentError> {
    given.allow(&[
        "item",
        "file",
        "times",
        "compare_item",
        "compare_file",
        "compare_source",
    ])?;
    let id = |key: &str| given.uuid(key, &format!("frames: {key} is a Library item ID"));
    let times = match given.given("times") {
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| refuse("frames: times is a list of seconds"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        None => {
            return Err(refuse(
                "frames needs times, the seconds to take frames at, such as [1, 2.5]",
            ));
        }
        Some(_) => return Err(refuse("frames: times is a list of seconds")),
    };
    let mut command = FramesCommand {
        item: id("item")?,
        file: given.string("file")?,
        times,
        compare_item: id("compare_item")?,
        compare_file: given.string("compare_file")?,
        compare_source: given.bool("compare_source")?,
    };
    if let Some(problem) = command.problem() {
        return Err(refuse(format!("frames: {problem}")));
    }
    command.file = command.file.as_deref().map(|path| local(path, working_directory));
    command.compare_file = command
        .compare_file
        .as_deref()
        .map(|path| local(path, working_directory));
    Ok(AgentCommand::Frames(command))
}

/// What names a `frames` command's uploads: it isn't a paid request, so its ID is made from its fingerprint.
pub(crate) fn frames_upload(command: &FramesCommand) -> Result<UploadRequest, AgentError> {
    let fingerprint = fingerprint::of(command).map_err(|_| refuse("frames: the parameters can't be read"))?;
    let request_id = format!("frames-{}", &fingerprint[..16]);
    Ok(UploadRequest {
        command: "frames".to_owned(),
        request_id,
        fingerprint,
        parameters: None,
    })
}

pub(crate) fn export(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&["items", "preset", "sidecar", "wait_seconds"])?;
    let Some(preset) = given
        .string("preset")?
        .as_deref()
        .and_then(AgentExportPreset::parse)
    else {
        return Err(refuse(format!(
            "export needs preset, one of {}",
            AgentExportPreset::names()
        )));
    };
    let command = ExportCommand {
        items: given.ids("items")?,
        preset,
        sidecar: given.optional_bool("sidecar")?,
        wait_seconds: given.int("wait_seconds")?,
    };
    if let Some(problem) = command.problem() {
        return Err(refuse(format!("export: {problem}")));
    }
    Ok(AgentCommand::Export(command))
}
