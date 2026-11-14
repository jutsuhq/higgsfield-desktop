//! The MCP tools, one per app command, in the order `tools/list` gives them. Arguments are `snake_case`, as MCP tools
//! usually are; the app's answer keeps its own `camelCase` JSON.

use crate::{
    arguments::{Arguments, refuse},
    inbox::Inbox,
};
use jutsu_agent::{
    AgentAmount, AgentCommand, AgentError, AgentJobAction, AgentPage, AgentSheet, AgentVideoRequest,
    CapabilitiesCommand, CreateVideoCommand, JobActionCommand, JobsCommand, QuoteCommand, ShowCommand, Uuid,
    fingerprint,
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::{fs, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Capabilities,
    Quote,
    CreateVideo,
    Jobs,
    JobAction,
    CreateImage,
    Elements,
    CreateElement,
    LibrarySearch,
    LibraryGet,
    LibraryDownload,
    LibraryMark,
    Enhance,
    DeliverySet,
    Export,
    Frames,
    Subtitles,
    Show,
}

impl Tool {
    /// Every tool, in the order `tools/list` gives them.
    pub const ALL: [Self; 18] = [
        Self::Capabilities,
        Self::Quote,
        Self::CreateVideo,
        Self::Jobs,
        Self::JobAction,
        Self::CreateImage,
        Self::Elements,
        Self::CreateElement,
        Self::LibrarySearch,
        Self::LibraryGet,
        Self::LibraryDownload,
        Self::LibraryMark,
        Self::Enhance,
        Self::DeliverySet,
        Self::Export,
        Self::Frames,
        Self::Subtitles,
        Self::Show,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Capabilities => "capabilities",
            Self::Quote => "quote",
            Self::CreateVideo => "create_video",
            Self::Jobs => "jobs",
            Self::JobAction => "job_action",
            Self::CreateImage => "create_image",
            Self::Elements => "elements",
            Self::CreateElement => "create_element",
            Self::LibrarySearch => "library_search",
            Self::LibraryGet => "library_get",
            Self::LibraryDownload => "library_download",
            Self::LibraryMark => "library_mark",
            Self::Enhance => "enhance",
            Self::DeliverySet => "delivery_set",
            Self::Export => "export",
            Self::Frames => "frames",
            Self::Subtitles => "subtitles",
            Self::Show => "show",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.name() == name)
    }

    /// The command for these arguments, or the tool error that tells the model what to fix. Checked here before
    /// anything reaches the app, and again by the app as it decodes the command. Relative paths are the agent's, from
    /// `working_directory`; `McpServer` hands the files to the app before it sends the command.
    pub fn command(
        self,
        arguments: &Map<String, Value>,
        working_directory: &Path,
    ) -> Result<AgentCommand, AgentError> {
        let given = Arguments::new(self.name(), arguments);
        match self {
            Self::Capabilities => capabilities(&given),
            Self::Quote => quote(&given),
            Self::CreateVideo => create_video(&given, working_directory),
            Self::CreateImage => crate::library_tools::create_image(&given, working_directory),
            Self::Jobs => jobs(&given),
            Self::JobAction => job_action(&given),
            Self::LibrarySearch => crate::library_tools::library_search(&given),
            Self::LibraryGet => crate::library_tools::library_get(&given),
            Self::Elements => crate::library_tools::elements(&given),
            Self::CreateElement => crate::library_tools::create_element(&given, working_directory),
            Self::LibraryDownload => crate::library_tools::library_download(&given),
            Self::LibraryMark => crate::library_tools::library_mark(&given),
            Self::Enhance => crate::library_tools::enhance(&given),
            Self::DeliverySet => crate::library_tools::delivery_set(&given),
            Self::Export => crate::library_tools::export(&given),
            Self::Frames => crate::library_tools::frames(&given, working_directory),
            Self::Subtitles => crate::library_tools::subtitles(&given),
            Self::Show => show(&given),
        }
    }
}

fn capabilities(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&["models", "model", "image_models", "styles", "voices"])?;
    let model = given.string("model")?;
    if let Some(problem) = CapabilitiesCommand::problem(model.as_deref()) {
        return Err(refuse(format!("capabilities: {problem}")));
    }
    let models = given.bool("models")?;
    Ok(AgentCommand::Capabilities(CapabilitiesCommand {
        models: models || model.is_some(),
        model,
        image_models: given.bool("image_models")?,
        styles: given.bool("styles")?,
        voices: given.bool("voices")?,
    }))
}

/// What `quote` and `create_video` both take.
pub(crate) const VIDEO_KEYS: [&str; 15] = [
    "mode",
    "model",
    "prompt",
    "source",
    "images",
    "style",
    "resolution",
    "draft",
    "length",
    "aspect_ratio",
    "audio",
    "variants",
    "elements",
    "title",
    "speech",
];

fn with_video_keys(extra: &[&'static str]) -> Vec<&'static str> {
    VIDEO_KEYS.iter().chain(extra).copied().collect()
}

fn quote(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&with_video_keys(&["image_count", "speech_length"]))?;
    let mut video = given.video()?;
    if video.speech.is_some() {
        return Err(refuse(
            "quote takes speech_length, the recording's seconds, not the speech file",
        ));
    }
    video.speech_length = given.number("speech_length")?;
    // A quote reads no file: only how many images there are counts.
    if !video.images.is_empty() && given.int("image_count")?.is_none() {
        video.image_count = Some(i64::try_from(video.images.len()).unwrap_or(i64::MAX));
    }
    video.images = Vec::new();
    if let Some(source) = &mut video.source {
        source.path = None;
    }
    video.title = None;
    if let Some(count) = given.int("image_count")? {
        video.image_count = Some(count);
    }
    if let Some(problem) = video.problem() {
        return Err(refuse(format!("quote: {problem}")));
    }
    Ok(AgentCommand::Quote(QuoteCommand { video }))
}

fn create_video(given: &Arguments, working_directory: &Path) -> Result<AgentCommand, AgentError> {
    given.allow(&with_video_keys(&["request_id", "max_cost", "wait_seconds"]))?;
    let request_id = given.request_id()?;
    let max_cost = given.max_cost()?;
    let wait = given.int("wait_seconds")?.unwrap_or(0);
    if let Some(problem) = JobsCommand::wait_problem(wait) {
        return Err(refuse(format!("create_video: {problem}")));
    }
    let mut video = given.video()?;
    if let Some(problem) = video.problem() {
        return Err(refuse(format!("create_video: {problem}")));
    }
    if video.image_count.is_some() {
        return Err(refuse("create_video takes images, the files, not a count"));
    }
    // The agent's paths as files on this computer, so the same files always give the same fingerprint.
    let local = |path: &str| {
        Inbox::resolve(path, working_directory)
            .to_string_lossy()
            .into_owned()
    };
    if let Some(source) = &mut video.source
        && let Some(path) = &source.path
    {
        source.path = Some(local(path));
    }
    video.images = video.images.iter().map(|path| local(path)).collect();
    video.speech = video.speech.as_deref().map(local);
    let fingerprint = video_fingerprint(&video, max_cost)?;
    let parameters = video_parameters(&video)?;
    Ok(AgentCommand::CreateVideo(CreateVideoCommand {
        request_id,
        fingerprint,
        parameters: Some(parameters),
        video,
        max_cost,
        wait_seconds: wait,
    }))
}

fn jobs(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&["ids", "limit", "wait_seconds", "cursor"])?;
    let ids = given
        .strings("ids")?
        .iter()
        .map(|text| Uuid::parse(text).ok_or_else(|| refuse("jobs: ids are job IDs (UUIDs)")))
        .collect::<Result<Vec<_>, _>>()?;
    let limit = given.int("limit")?;
    let wait = given.int("wait_seconds")?.unwrap_or(0);
    let cursor = given.string("cursor")?;
    let count = i64::try_from(ids.len()).unwrap_or(i64::MAX);
    if let Some(problem) = JobsCommand::problem(count, limit, wait, cursor.as_deref()) {
        return Err(refuse(format!("jobs: {problem}")));
    }
    Ok(AgentCommand::Jobs(JobsCommand {
        ids,
        limit,
        wait_seconds: wait,
        cursor,
    }))
}

fn job_action(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&["action", "job"])?;
    let Some(action) = given.string("action")?.as_deref().and_then(AgentJobAction::parse) else {
        return Err(refuse(format!(
            "job_action: action is one of {}",
            AgentJobAction::names()
        )));
    };
    let job = given.string("job")?;
    if let Some(problem) = JobActionCommand::problem(action, job.is_some()) {
        return Err(refuse(format!("job_action: {problem}")));
    }
    let job = job
        .map(|text| Uuid::parse(&text).ok_or_else(|| refuse("job_action: job is a job ID")))
        .transpose()?;
    Ok(AgentCommand::JobAction(JobActionCommand { action, job }))
}

fn show(given: &Arguments) -> Result<AgentCommand, AgentError> {
    given.allow(&["page", "job", "item", "sheet"])?;
    let (page, job) = (given.string("page")?, given.string("job")?);
    let (item, sheet) = (given.string("item")?, given.string("sheet")?);
    if let Some(problem) =
        ShowCommand::problem(page.is_some(), job.is_some(), item.is_some(), sheet.is_some())
    {
        return Err(refuse(problem));
    }
    if let Some(page) = page {
        let Some(page) = AgentPage::parse(&page) else {
            return Err(refuse(format!("show: page is one of {}", AgentPage::names())));
        };
        return Ok(AgentCommand::Show(ShowCommand::page(page)));
    }
    if let (Some(item), Some(sheet)) = (item, sheet) {
        let Some(id) = Uuid::parse(&item) else {
            return Err(refuse("show: item is a Library item ID"));
        };
        let Some(sheet) = AgentSheet::parse(&sheet) else {
            return Err(refuse(format!("show: sheet is one of {}", AgentSheet::names())));
        };
        return Ok(AgentCommand::Show(ShowCommand::sheet(sheet, id)));
    }
    let Some(id) = job.as_deref().and_then(Uuid::parse) else {
        return Err(refuse("show: job is a job ID (a UUID)"));
    };
    Ok(AgentCommand::Show(ShowCommand::job(id)))
}

/// The request's files, in a fixed order: the source, the images and the speech.
pub(crate) fn files_of(video: &AgentVideoRequest) -> Vec<String> {
    let source = video.source.as_ref().and_then(|source| source.path.clone());
    source
        .into_iter()
        .chain(video.images.iter().cloned())
        .chain(video.speech.clone())
        .collect()
}

/// The files' sizes, `None` for one that can't be read: a file that changed size makes another request.
pub(crate) fn sizes(files: &[String]) -> Vec<Option<u64>> {
    files
        .iter()
        .map(|path| {
            fs::metadata(path)
                .ok()
                .filter(fs::Metadata::is_file)
                .map(|metadata| metadata.len())
        })
        .collect()
}

/// What makes two `create_video` requests the same: the parameters, the files by path and size, and the most the
/// agent agreed to pay. A file that changed size since makes it another request.
pub(crate) fn video_fingerprint(
    video: &AgentVideoRequest,
    max_cost: AgentAmount,
) -> Result<String, AgentError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Fingerprinted<'a> {
        video: &'a AgentVideoRequest,
        sizes: Vec<Option<u64>>,
        max_cost: AgentAmount,
    }
    let sizes = sizes(&files_of(video));
    fingerprint::of(&Fingerprinted {
        video,
        sizes,
        max_cost,
    })
    .map_err(|_| refuse("create_video: the parameters can't be read"))
}

/// The same as `video_fingerprint` without `max_cost`: what a `create_video` sent again with a higher `max_cost`
/// shares with the request it raises.
pub(crate) fn video_parameters(video: &AgentVideoRequest) -> Result<String, AgentError> {
    #[derive(Serialize)]
    struct Parameters<'a> {
        video: &'a AgentVideoRequest,
        sizes: Vec<Option<u64>>,
    }
    let sizes = sizes(&files_of(video));
    fingerprint::of(&Parameters { video, sizes })
        .map_err(|_| refuse("create_video: the parameters can't be read"))
}

/// The fingerprint of another paid tool's request: the tool, its parameters, its files by size and `max_cost`. The
/// tool is part of it, so one request ID never means two tools' requests. `max_cost` is `None` for a free run, and is
/// then left out.
pub(crate) fn tool_fingerprint<T: Serialize>(
    tool: &str,
    fields: &T,
    files: &[String],
    max_cost: Option<AgentAmount>,
) -> Result<String, AgentError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ToolFingerprint<'a, T: Serialize> {
        tool: &'a str,
        fields: &'a T,
        sizes: Vec<Option<u64>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_cost: Option<AgentAmount>,
    }
    fingerprint::of(&ToolFingerprint {
        tool,
        fields,
        sizes: sizes(files),
        max_cost,
    })
    .map_err(|_| refuse(format!("{tool}: the parameters can't be read")))
}
