//! What `tools/list` shows of each tool: its name, what it does, its input and output schemas and hints.

use crate::{
    descriptions,
    schemas::{max_cost_schema, output_schema, raw_values, request_id_schema},
    tools::Tool,
};
use jutsu_agent::{
    AgentCulling, AgentDeliveryAction, AgentElementNames, AgentEnhancement, AgentExportPreset,
    AgentJobAction, AgentLibraryKind, AgentPage, AgentSheet, AgentSmoothMotion, AgentSource,
    AgentSubtitleEngine, AgentSubtitleFormat, AgentSubtitlesAction, AgentVerdict, CreateElementCommand,
    DeliverySetCommand, ElementsCommand, EnhanceCommand, ExportCommand, FramesCommand, JobsCommand,
    LibraryDownloadCommand, LibraryGetCommand, LibraryMarkCommand, LibrarySearchCommand, SubtitlesCommand,
    elements, library, video,
};
use serde_json::{Map, Value, json};

impl Tool {
    /// The tool as `tools/list` shows it. `structured`: whether the client reads `structuredContent` (2025-06-18 on);
    /// only then is there an output schema to promise it.
    pub fn definition(self, structured: bool) -> Value {
        let mut definition = Map::new();
        definition.insert("name".to_owned(), Value::from(self.name()));
        definition.insert(
            "title".to_owned(),
            Value::from(descriptions::on_this_computer(self.title())),
        );
        definition.insert("description".to_owned(), Value::from(self.about()));
        definition.insert("inputSchema".to_owned(), on_this_computer(self.input_schema()));
        definition.insert("annotations".to_owned(), self.annotations());
        if structured {
            definition.insert("outputSchema".to_owned(), output_schema(self));
        }
        Value::Object(definition)
    }

    /// What the tool does, for the model.
    pub fn about(self) -> String {
        descriptions::about(self)
    }

    fn title(self) -> &'static str {
        match self {
            Self::Capabilities => "Jutsu Studio capabilities",
            Self::Quote => "Quote a video",
            Self::CreateVideo => "Create a video",
            Self::CreateImage => "Create images",
            Self::Elements => "Characters and elements",
            Self::CreateElement => "Create an element",
            Self::Jobs => "Jobs",
            Self::JobAction => "Act on a job or the queue",
            Self::LibrarySearch => "Search the Library",
            Self::LibraryGet => "Library items in full",
            Self::LibraryDownload => "Download from Higgsfield",
            Self::LibraryMark => "Keep, reject or star",
            Self::Enhance => "Enhance a result",
            Self::DeliverySet => "Delivery sets",
            Self::Export => "Export for editing",
            Self::Frames => "Still frames and contact sheet",
            Self::Subtitles => "Subtitles on this Mac",
            Self::Show => "Show in Jutsu Studio",
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one schema per tool, as the tools are listed"
    )]
    fn input_schema(self) -> Value {
        let mut schema = Map::new();
        schema.insert("type".to_owned(), Value::from("object"));
        schema.insert("additionalProperties".to_owned(), Value::Bool(false));
        let (properties, required): (Value, &[&str]) = match self {
            Self::Capabilities => (
                json!({
                    "models": { "type": "boolean", "description": "Adds the video models to the answer." },
                    "model": {
                        "type": "string", "pattern": "^[A-Za-z0-9_-][A-Za-z0-9._-]{0,63}$",
                        "description": "Only this model, by its ID.",
                    },
                    "image_models": { "type": "boolean", "description": "Adds the image models create_image takes." },
                    "styles": { "type": "boolean", "description": "Adds Restyle's style IDs." },
                    "voices": { "type": "boolean", "description": "Adds the voices change_voice takes." },
                }),
                &[],
            ),
            Self::Quote => (video_properties(false), &["mode"]),
            Self::CreateVideo => {
                let mut properties = video_properties(true);
                if let Some(properties) = properties.as_object_mut() {
                    properties.insert("request_id".to_owned(), request_id_schema());
                    properties.insert("max_cost".to_owned(), max_cost_schema());
                    properties.insert(
                        "wait_seconds".to_owned(),
                        json!({
                            "type": "integer", "minimum": 0, "maximum": JobsCommand::MAX_WAIT_SECONDS,
                            "description": "Wait up to this long for the job's price to be known, or for it to be held.",
                        }),
                    );
                }
                (properties, &["request_id", "mode", "max_cost"])
            }
            Self::CreateImage => (
                json!({
                    "request_id": request_id_schema(), "max_cost": max_cost_schema(),
                    "model": { "type": "string", "description": "An image model ID from capabilities." },
                    "prompt": prompt_schema(),
                    "aspect_ratio": { "type": "string", "description": "One the model takes: 1:1, 16:9, 9:16…" },
                    "count": {
                        "type": "integer", "minimum": 1, "maximum": library::MAX_COUNT,
                        "description": "Images to make, one the model offers; 1 when left out.",
                    },
                    // No maxItems: the limit is the model's, which capabilities' image_models give.
                    "inputs": {
                        "type": "array", "items": { "type": "string" },
                        "description": "Images on this Mac to edit, in order, up to the model's maxInputs.",
                    },
                    "input": { "type": "string", "description": "One image to edit: the same as inputs with one item." },
                    "elements": elements_schema(),
                    "title": title_schema(),
                }),
                &["request_id", "prompt", "max_cost"],
            ),
            Self::LibrarySearch => (
                json!({
                    "query": { "type": "string", "maxLength": LibrarySearchCommand::MAX_QUERY_LENGTH },
                    "kind": { "type": "string", "enum": raw_values(AgentLibraryKind::ALL, AgentLibraryKind::as_str) },
                    "starred": { "type": "boolean" },
                    "verdict": { "type": "string", "enum": raw_values(AgentVerdict::ALL, AgentVerdict::as_str) },
                    "include_rejected": { "type": "boolean" },
                    "model": {
                        "type": "string", "maxLength": LibrarySearchCommand::MAX_MODEL_LENGTH,
                        "description": "A video model ID or an image model's name.",
                    },
                    "mode": { "type": "string", "enum": video::MODES },
                    "from": { "type": "string", "format": "date-time" },
                    "to": { "type": "string", "format": "date-time" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": LibrarySearchCommand::MAX_LIMIT },
                    "cursor": { "type": "string", "description": "nextCursor of the page before." },
                    "include_higgsfield": {
                        "type": "boolean", "description": "Also the account's Higgsfield history, not on this Mac yet.",
                    },
                    "refresh_higgsfield": {
                        "type": "boolean", "description": "Load the history from Higgsfield again first; free.",
                    },
                }),
                &[],
            ),
            Self::LibraryGet => (
                json!({
                    "ids": {
                        "type": "array", "minItems": 1, "maxItems": LibraryGetCommand::MAX_IDS,
                        "items": { "type": "string", "format": "uuid" }, "description": "Library item IDs.",
                    },
                }),
                &["ids"],
            ),
            Self::LibraryDownload => (
                json!({
                    "ids": {
                        "type": "array", "minItems": 1, "maxItems": LibraryDownloadCommand::MAX_IDS,
                        "items": { "type": "string", "format": "uuid" }, "description": "Library item IDs onHiggsfield.",
                    },
                }),
                &["ids"],
            ),
            Self::LibraryMark => (
                json!({
                    "ids": {
                        "type": "array", "minItems": 1, "maxItems": LibraryMarkCommand::MAX_IDS,
                        "items": { "type": "string", "format": "uuid" }, "description": "Library item IDs.",
                    },
                    "verdict": { "type": "string", "enum": raw_values(AgentCulling::ALL, AgentCulling::as_str) },
                    "star": { "type": "boolean" },
                }),
                &["ids"],
            ),
            Self::Enhance => {
                let mut max_cost = max_cost_schema();
                if let Some(fields) = max_cost.as_object_mut() {
                    fields.insert(
                        "description".to_owned(),
                        Value::from("On Higgsfield, the most you agree to pay; a run on this Mac is free and needs none."),
                    );
                }
                (
                    json!({
                        "request_id": request_id_schema(), "max_cost": max_cost,
                        "item": {
                            "type": "string", "format": "uuid",
                            "description": "A video's Library ID, or an image's for upscale with engine mac.",
                        },
                        "operation": {
                            "type": "string", "enum": raw_values(AgentEnhancement::ALL, AgentEnhancement::as_str),
                        },
                        "target": {
                            "type": "string",
                            "description": "upscale: 1080p, 2k or 4k; with engine mac also a factor such as 2x or 2.5x.",
                        },
                        "frame_rate": { "type": "integer", "enum": [24, 30, 60] },
                        "preset": {
                            "type": "string", "enum": ["aigc", "common", "ugc", "short_series", "old_film"],
                            "description": "upscale on Higgsfield: what the upscaler tunes itself for; aigc when left out.",
                        },
                        "engine": {
                            "type": "string", "enum": ["standard", EnhanceCommand::MAC_ENGINE],
                            "description": "upscale: standard on Higgsfield (paid), mac on this Mac (free, 4×).",
                        },
                        "mode": {
                            "type": "string", "enum": raw_values(AgentSmoothMotion::ALL, AgentSmoothMotion::as_str),
                            "description": "smooth_motion: 60fps keeps length and sound; slow_2x and slow_4x drop the sound.",
                        },
                        "aspect_ratio": { "type": "string", "enum": ["9:16", "1:1", "16:9"] },
                        "language": { "type": "string", "description": "An ISO 639-3 code, such as fra." },
                        "voice": {
                            "type": "string", "minLength": 1, "maxLength": EnhanceCommand::MAX_VOICE_LENGTH,
                            "description": "A voice ID from capabilities.",
                        },
                    }),
                    &["request_id", "item", "operation"],
                )
            }
            Self::DeliverySet => (
                json!({
                    "action": {
                        "type": "string", "enum": raw_values(AgentDeliveryAction::ALL, AgentDeliveryAction::as_str),
                    },
                    "name": {
                        "type": "string", "maxLength": DeliverySetCommand::MAX_NAME_LENGTH,
                        "description": "create: the set's name.",
                    },
                    "items": {
                        "type": "array", "maxItems": DeliverySetCommand::MAX_ITEMS,
                        "items": { "type": "string", "format": "uuid" }, "description": "create: Library item IDs.",
                    },
                    "set": { "type": "string", "format": "uuid", "description": "export: the set's ID." },
                    "folder": { "type": "string", "description": "export: one of list's folders." },
                    "wait_seconds": export_wait_schema(),
                }),
                &["action"],
            ),
            Self::Export => (
                json!({
                    "items": {
                        "type": "array", "minItems": 1, "maxItems": ExportCommand::MAX_ITEMS,
                        "items": { "type": "string", "format": "uuid" }, "description": "Video Library IDs.",
                    },
                    "preset": {
                        "type": "string", "enum": raw_values(AgentExportPreset::ALL, AgentExportPreset::as_str),
                    },
                    "sidecar": { "type": "boolean", "description": "Also write the JSON sidecar; true when left out." },
                    "wait_seconds": export_wait_schema(),
                }),
                &["items", "preset"],
            ),
            Self::Jobs => (
                json!({
                    "ids": {
                        "type": "array", "maxItems": JobsCommand::MAX_IDS,
                        "items": { "type": "string", "format": "uuid" }, "description": "Job IDs.",
                    },
                    "limit": {
                        "type": "integer", "minimum": 1, "maximum": JobsCommand::MAX_LIMIT,
                        "description": "Without ids: how many of the latest agent jobs, newest first.",
                    },
                    "wait_seconds": {
                        "type": "integer", "minimum": 0, "maximum": JobsCommand::MAX_WAIT_SECONDS,
                        "description": "Wait up to this long for the jobs to settle.",
                    },
                    "cursor": { "type": "string", "description": "Without ids: nextCursor of the page before." },
                }),
                &[],
            ),
            Self::JobAction => (
                json!({
                    "action": { "type": "string", "enum": raw_values(AgentJobAction::ALL, AgentJobAction::as_str) },
                    "job": {
                        "type": "string", "format": "uuid", "description": "The job, for cancel, retry and run_next.",
                    },
                }),
                &["action"],
            ),
            Self::Frames => (
                json!({
                    "item": { "type": "string", "format": "uuid", "description": "A Library item, a video or an image." },
                    "file": { "type": "string", "description": "Else a video or an image on this Mac." },
                    "times": {
                        "type": "array", "minItems": 1, "maxItems": FramesCommand::MAX_TIMES,
                        "items": { "type": "number", "minimum": 0 }, "description": "Seconds from the start, in order.",
                    },
                    "compare_item": { "type": "string", "format": "uuid", "description": "Another Library item." },
                    "compare_file": { "type": "string", "description": "Another video or image on this Mac." },
                    "compare_source": { "type": "boolean", "description": "The item's own source clip." },
                }),
                &["times"],
            ),
            Self::Subtitles => (
                json!({
                    "action": {
                        "type": "string", "enum": raw_values(AgentSubtitlesAction::ALL, AgentSubtitlesAction::as_str),
                    },
                    "item": { "type": "string", "format": "uuid", "description": "A video's Library ID." },
                    "request_id": request_id_schema(),
                    "language": {
                        "type": "string", "maxLength": SubtitlesCommand::MAX_LANGUAGE_LENGTH,
                        "description": "make: en-US, de… from capabilities' onDevice.subtitles; the person's when left out.",
                    },
                    "engine": {
                        "type": "string", "enum": raw_values(AgentSubtitleEngine::ALL, AgentSubtitleEngine::as_str),
                        "description": "make: the speech model; the person's default from Settings when left out.",
                    },
                    "format": {
                        "type": "string", "enum": raw_values(AgentSubtitleFormat::ALL, AgentSubtitleFormat::as_str),
                        "description": "export: the file's format; get: adds the file's text.",
                    },
                }),
                &["action", "item"],
            ),
            Self::Elements => (
                json!({
                    "query": {
                        "type": "string", "maxLength": ElementsCommand::MAX_QUERY_LENGTH,
                        "description": "Words of the name.",
                    },
                    "category": { "type": "string", "enum": elements::CATEGORIES },
                    "limit": { "type": "integer", "minimum": 1, "maximum": ElementsCommand::MAX_LIMIT },
                    "cursor": { "type": "string", "description": "nextCursor of the page before." },
                }),
                &[],
            ),
            Self::CreateElement => (
                json!({
                    "name": {
                        "type": "string", "maxLength": CreateElementCommand::MAX_NAME_LENGTH,
                        "description": "What elements and create_video's and create_image's elements call it.",
                    },
                    "category": {
                        "type": "string", "enum": elements::CATEGORIES,
                        "description": "character when left out.",
                    },
                    "images": {
                        "type": "array", "minItems": 1, "maxItems": CreateElementCommand::MAX_IMAGES,
                        "items": { "type": "string" },
                        "description": "Clear images of the subject on this Mac, in order; the first is its face or front.",
                    },
                    "request_id": request_id_schema(),
                }),
                &["request_id", "name", "images"],
            ),
            // Exactly one of the two, said in words and checked on the call: some model APIs refuse oneOf at the
            // top of an input schema.
            Self::Show => (
                json!({
                    "page": {
                        "type": "string", "enum": raw_values(AgentPage::ALL, AgentPage::as_str),
                        "description": "A page of the sidebar.",
                    },
                    "job": { "type": "string", "format": "uuid", "description": "A job ID." },
                    "item": { "type": "string", "format": "uuid", "description": "A Library item ID, with sheet." },
                    "sheet": {
                        "type": "string", "enum": raw_values(AgentSheet::ALL, AgentSheet::as_str),
                        "description": "upscale_on_mac: the item's Upscale on This Mac sheet; subtitles: its Subtitles sheet.",
                    },
                }),
                &[],
            ),
        };
        schema.insert("properties".to_owned(), properties);
        if !required.is_empty() {
            schema.insert("required".to_owned(), json!(required));
        }
        Value::Object(schema)
    }

    fn annotations(self) -> Value {
        let (read_only, destructive, idempotent, open_world) = match self {
            Self::Capabilities | Self::Quote | Self::Jobs | Self::LibraryGet => (true, false, true, false),
            // Reads the account's elements on Higgsfield; refresh_higgsfield reads the account's history from
            // Higgsfield.
            Self::Elements | Self::LibrarySearch => (true, false, true, true),
            // Adds files to the Library from Higgsfield; one downloaded already is left as it is. Spends, but the
            // same request_id never queues twice; it reaches Higgsfield. create_element is free, and the same
            // request_id never makes a second element. The same marks again change nothing; a star may also like the
            // result on Higgsfield.
            Self::LibraryDownload
            | Self::CreateVideo
            | Self::CreateImage
            | Self::Enhance
            | Self::CreateElement
            | Self::LibraryMark => (false, false, true, true),
            // Cancel stops a job, retry may queue another attempt.
            Self::JobAction => (false, true, false, true),
            // Writes new files on this computer, never over one; each call writes new ones. subtitles make saves cues
            // on the item and export writes a new file, both on this computer; free.
            Self::DeliverySet | Self::Export | Self::Subtitles => (false, false, false, false),
            // Writes new files likewise; compare_source may read a history item's source from Higgsfield.
            Self::Frames => (false, false, false, true),
            // Changes what the window shows, nothing else.
            Self::Show => (false, false, true, false),
        };
        json!({
            "readOnlyHint": read_only, "destructiveHint": destructive, "idempotentHint": idempotent,
            "openWorldHint": open_world,
        })
    }
}

/// The macOS app's words for this computer in every string of a schema: its descriptions.
fn on_this_computer(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(descriptions::on_this_computer(&text)),
        Value::Array(items) => Value::Array(items.into_iter().map(on_this_computer).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, on_this_computer(value)))
                .collect(),
        ),
        other => other,
    }
}

fn title_schema() -> Value {
    json!({
        "type": "string", "maxLength": video::MAX_TITLE_LENGTH,
        "description": "What the Queue, the Library and file names call it; the first 80 characters, on one line.",
    })
}

fn prompt_schema() -> Value {
    json!({ "type": "string", "maxLength": video::MAX_PROMPT_LENGTH })
}

fn export_wait_schema() -> Value {
    json!({
        "type": "integer", "minimum": 0, "maximum": JobsCommand::MAX_WAIT_SECONDS,
        "description": "Wait up to this long for the export to finish before answering; it runs on either way.",
    })
}

fn elements_schema() -> Value {
    json!({
        "type": "array", "maxItems": AgentElementNames::MAX_NAMES,
        "items": { "type": "string", "maxLength": AgentElementNames::MAX_NAME_LENGTH },
        "description": "The account's elements by name, from the elements tool; their images go first.",
    })
}

/// The source clip's range, and for `create_video` its file.
fn source_properties(files: bool) -> Value {
    let mut source = json!({
        "start": {
            "type": "number", "minimum": 0, "maximum": AgentSource::MAX_SECONDS,
            "description": "Seconds from the start of the file.",
        },
        "end": {
            "type": "number", "minimum": 0, "maximum": AgentSource::MAX_SECONDS,
            "description": "Where the job's range ends, in seconds.",
        },
    });
    if files && let Some(fields) = source.as_object_mut() {
        fields.insert(
            "path".to_owned(),
            json!({ "type": "string", "description": "The clip on this Mac." }),
        );
    }
    source
}

/// The video fields of `quote` and `create_video`; `files` for paths to send, else counts and lengths only.
fn video_properties(files: bool) -> Value {
    let source_description = if files {
        "The source clip; left out, a model that can makes the clip from the prompt and images."
    } else {
        "The source clip's range; end is needed."
    };
    let mut properties = json!({
        "mode": { "type": "string", "enum": video::MODES },
        "model": { "type": "string", "description": "A model ID from capabilities; the mode's default if left out." },
        "prompt": prompt_schema(),
        "source": {
            "type": "object", "additionalProperties": false, "properties": source_properties(files),
            "description": source_description,
        },
        // No maxItems: the limit is the model's in its mode, which capabilities and quote give.
        "images": {
            "type": "array", "items": { "type": "string" },
            "description": "Reference images on this Mac, in order, up to the model's maxImages (quote).",
        },
        "style": { "type": "string", "format": "uuid", "description": "Restyle's style ID." },
        "resolution": { "type": "string", "description": "One from capabilities, such as 720p." },
        "draft": { "type": "boolean", "description": "A cheaper lower-resolution preview, on models with drafts." },
        "length": {
            "type": "number", "exclusiveMinimum": 0, "maximum": video::MAX_LENGTH,
            "description": "Seconds a clip without a source runs, or extend adds.",
        },
        "aspect_ratio": {
            "type": "string",
            "description": "A clip without a source: 16:9, 9:16, 1:1…; with one it keeps the source's orientation.",
        },
        "audio": { "type": "string", "enum": video::AUDIO_OPTIONS },
        "variants": {
            "type": "integer", "minimum": 1, "maximum": video::MAX_VARIANTS,
            "description": "Videos one submit makes, up to quote's maxVariants.",
        },
        "elements": elements_schema(),
    });
    if let Some(fields) = properties.as_object_mut() {
        if files {
            fields.insert("title".to_owned(), title_schema());
            fields.insert(
                "speech".to_owned(),
                json!({
                    "type": "string",
                    "description": "A speech recording on this Mac (MP3, WAV, M4A…), for a model whose takesSpeech is \
                                    true (capabilities): the characters say it, and its length sets the clip's.",
                }),
            );
        } else {
            fields.insert(
                "image_count".to_owned(),
                json!({
                    "type": "integer", "minimum": 0, "maximum": video::PROTOCOL_MAX_IMAGES,
                    "description": "How many reference images there will be.",
                }),
            );
            fields.insert(
                "speech_length".to_owned(),
                json!({
                    "type": "number", "exclusiveMinimum": 0, "maximum": video::MAX_LENGTH,
                    "description": "Seconds the speech recording runs, for a model whose takesSpeech is true.",
                }),
            );
        }
    }
    properties
}
