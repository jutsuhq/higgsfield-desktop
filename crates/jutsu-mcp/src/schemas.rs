//! The output schemas `tools/list` promises, and the shapes the tools share.

use crate::tools::Tool;
use jutsu_agent::{AgentDownloadStatus, AgentInputRole, AgentPage};
use serde_json::{Map, Value, json};

/// Every raw value of a wire enum, in order, as a JSON array.
pub(crate) fn raw_values<T: Copy>(all: &[T], raw: fn(T) -> &'static str) -> Value {
    Value::Array(all.iter().map(|case| Value::from(raw(*case))).collect())
}

fn kind(name: &str) -> Value {
    json!({ "type": name })
}

fn list_of(items: Value) -> Value {
    let mut list = json!({ "type": "array" });
    list["items"] = items;
    list
}

/// The answer's shape. Nothing is required at the top, so a tool error, `{"error": {…}}`, fits it too.
#[allow(
    clippy::too_many_lines,
    reason = "one schema per tool, as the tools are listed"
)]
pub(crate) fn output_schema(tool: Tool) -> Value {
    let mut properties = Map::new();
    let mut set = |key: &str, value: Value| {
        properties.insert(key.to_owned(), value);
    };
    set("error", error_schema());
    match tool {
        Tool::Capabilities => {
            set("protocolVersion", kind("integer"));
            set("appVersion", kind("string"));
            set("helperVersion", kind("string"));
            set("helperNote", kind("string"));
            set("agentsAllowed", kind("boolean"));
            set(
                "window",
                json!({ "type": "object", "properties": { "open": kind("boolean"), "menuBarMode": kind("boolean") } }),
            );
            set(
                "connection",
                json!({
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "enum": ["none", "apiKey", "account"] },
                        "plan": kind("string"), "credits": kind("integer"),
                    },
                }),
            );
            set(
                "budget",
                json!({
                    "type": "object",
                    "properties": {
                        "unit": { "type": "string", "enum": ["usd_cents", "credits"] },
                        "dailyLimit": kind("integer"), "spentToday": kind("integer"),
                        "reservedToday": kind("integer"), "remaining": kind("integer"),
                        "unlimitedAllowed": kind("boolean"), "dailyCap": kind("integer"),
                        "capTakenToday": kind("integer"),
                    },
                }),
            );
            set(
                "toolGroups",
                list_of(json!({
                    "type": "object",
                    "properties": { "name": kind("string"), "tools": list_of(kind("string")) },
                })),
            );
            set(
                "models",
                list_of(json!({
                    "type": "object",
                    "properties": {
                        "id": kind("string"), "title": kind("string"),
                        "modes": list_of(kind("string")),
                        "resolutions": list_of(kind("string")),
                        "available": kind("boolean"), "details": kind("object"),
                    },
                })),
            );
            set(
                "imageModels",
                list_of(json!({
                    "type": "object",
                    "properties": {
                        "id": kind("string"), "title": kind("string"),
                        "counts": list_of(kind("integer")),
                        "aspectRatios": list_of(kind("string")),
                        "input": { "type": "string", "enum": ["none", "optional", "required"] },
                        "maxInputs": kind("integer"), "maxInputsNote": kind("string"),
                    },
                })),
            );
            set("imageModelsNote", kind("string"));
            let named =
                json!({ "type": "object", "properties": { "id": kind("string"), "name": kind("string") } });
            set("styles", list_of(named.clone()));
            set("voices", list_of(named));
            let engine = json!({
                "type": "object",
                "properties": {
                    "available": kind("boolean"), "maxLongSide": kind("integer"),
                    "maxShortSide": kind("integer"),
                    "model": {
                        "type": "string", "enum": ["none", "needs_consent", "downloading", "ready", "agreed"],
                    },
                },
            });
            let speech = json!({
                "type": "object",
                "properties": {
                    "engine": kind("string"), "available": kind("boolean"),
                    "languages": list_of(kind("string")),
                    "needsConsent": list_of(kind("string")),
                    "model": {
                        "type": "string",
                        "enum": ["needs_consent", "agreed", "not_downloaded", "downloading", "ready"],
                    },
                },
            });
            set(
                "onDevice",
                json!({
                    "type": "object",
                    "properties": {
                        "smoothMotion": engine.clone(), "modes": list_of(kind("string")),
                        "upscaleVideo": engine.clone(), "upscaleImage": engine,
                        "subtitles": {
                            "type": "object",
                            "properties": { "defaultEngine": kind("string"), "engines": list_of(speech) },
                        },
                    },
                }),
            );
        }
        Tool::Quote => {
            set("price", amount_schema());
            set("isExact", kind("boolean"));
            set("mode", kind("string"));
            set("model", kind("string"));
            set("resolution", kind("string"));
            set("duration", kind("number"));
            set("remainingToday", kind("integer"));
            set("paysUnlimited", kind("boolean"));
            set("blocker", kind("string"));
            set("priceNote", kind("string"));
            set("maxVariants", kind("integer"));
            set("variantsUnavailable", kind("string"));
            set("images", kind("integer"));
            set("minImages", kind("integer"));
            set("maxImages", kind("integer"));
            set("aspectRatio", kind("string"));
            set("frameNote", kind("string"));
        }
        Tool::CreateVideo | Tool::CreateImage | Tool::Enhance => {
            set("requestId", kind("string"));
            set("known", kind("boolean"));
            set("jobs", list_of(job_schema()));
            set(
                "references",
                list_of(json!({
                    "type": "object",
                    "properties": { "tag": kind("string"), "element": kind("string"), "file": kind("string") },
                })),
            );
            if tool == Tool::CreateVideo {
                set("timedOut", kind("boolean"));
            }
        }
        Tool::Elements => {
            set(
                "items",
                list_of(json!({
                    "type": "object",
                    "properties": {
                        "id": kind("string"), "name": kind("string"), "category": kind("string"),
                        "imageCount": kind("integer"),
                    },
                })),
            );
            set("nextCursor", kind("string"));
        }
        Tool::CreateElement => {
            set("requestId", kind("string"));
            set("known", kind("boolean"));
            set("jobs", list_of(job_schema()));
            set(
                "element",
                json!({
                    "type": "object",
                    "properties": {
                        "id": kind("string"), "name": kind("string"), "category": kind("string"),
                        "imageCount": kind("integer"), "status": kind("string"),
                    },
                }),
            );
        }
        Tool::Jobs => {
            set("jobs", list_of(job_schema()));
            set("timedOut", kind("boolean"));
            set("nextCursor", kind("string"));
        }
        Tool::LibrarySearch => {
            set("items", list_of(item_schema()));
            set("nextCursor", kind("string"));
        }
        Tool::LibraryGet => {
            set(
                "items",
                list_of(json!({
                    "type": "object",
                    "properties": {
                        "item": item_schema(), "inputs": list_of(input_schema()),
                        "settings": settings_schema(), "next": kind("string"),
                        "madeFrom": {
                            "type": "object",
                            "properties": {
                                "item": kind("string"), "versionId": kind("string"),
                                "version": kind("integer"), "file": kind("string"),
                            },
                        },
                    },
                })),
            );
            set("notFound", list_of(kind("string")));
            set("next", kind("string"));
        }
        Tool::LibraryDownload => {
            set(
                "items",
                list_of(json!({
                    "type": "object",
                    "properties": {
                        "id": kind("string"),
                        "status": {
                            "type": "string",
                            "enum": raw_values(AgentDownloadStatus::ALL, AgentDownloadStatus::as_str),
                        },
                        "file": kind("string"), "message": kind("string"),
                    },
                })),
            );
        }
        Tool::LibraryMark => {
            set(
                "items",
                list_of(json!({
                    "type": "object",
                    "properties": { "id": kind("string"), "verdict": kind("string"), "starred": kind("boolean") },
                })),
            );
            set("changed", kind("integer"));
        }
        Tool::DeliverySet => {
            set("action", kind("string"));
            set("set", delivery_set_schema());
            set("sets", list_of(delivery_set_schema()));
            set("folders", list_of(kind("string")));
            set("job", job_schema());
            set("timedOut", kind("boolean"));
        }
        Tool::Export => {
            set("job", job_schema());
            set("timedOut", kind("boolean"));
        }
        Tool::Frames => {
            set("folder", kind("string"));
            set(
                "frames",
                list_of(json!({
                    "type": "object",
                    "properties": {
                        "clip": { "type": "string", "enum": ["main", "compare"] }, "time": kind("number"),
                        "file": kind("string"),
                    },
                })),
            );
            set("sheet", kind("string"));
        }
        Tool::Subtitles => {
            // make answers as enhance does; get and export with the version's subtitles.
            set("requestId", kind("string"));
            set("known", kind("boolean"));
            set("jobs", list_of(job_schema()));
            set("item", kind("string"));
            set("versionId", kind("string"));
            set("language", kind("string"));
            set("model", kind("string"));
            set(
                "cues",
                list_of(json!({
                    "type": "object",
                    "properties": { "start": kind("number"), "end": kind("number"), "text": kind("string") },
                })),
            );
            set("text", kind("string"));
            set("file", kind("string"));
        }
        Tool::JobAction => {
            set("action", kind("string"));
            set("job", job_schema());
        }
        Tool::Show => {
            set(
                "page",
                json!({ "type": "string", "enum": raw_values(AgentPage::ALL, AgentPage::as_str) }),
            );
            set("job", kind("string"));
            set("item", kind("string"));
            set("sheet", kind("string"));
        }
    }
    json!({ "type": "object", "properties": properties })
}

pub(crate) fn amount_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "amount": kind("integer"), "unit": { "type": "string", "enum": ["usd_cents", "credits"] } },
    })
}

pub(crate) fn job_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": kind("string"), "state": kind("string"), "pauseReason": kind("string"),
            "hold": kind("string"), "needsPerson": kind("boolean"), "progress": kind("number"),
            "price": amount_schema(), "isPriceExact": kind("boolean"), "charged": kind("boolean"),
            "error": kind("string"), "result": kind("string"),
            "results": list_of(kind("string")), "kind": kind("string"),
            "requestId": kind("string"),
            "client": kind("string"), "mode": kind("string"), "model": kind("string"),
            "folder": kind("string"), "files": list_of(export_file_schema()),
            "set": kind("string"), "operation": kind("string"), "item": kind("string"),
            "estimatedSeconds": kind("number"), "width": kind("integer"), "height": kind("integer"),
            "duration": kind("number"), "frameNote": kind("string"), "next": kind("string"),
            "createdAt": kind("string"), "finishedAt": kind("string"),
        },
    })
}

fn item_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": kind("string"), "kind": kind("string"), "file": kind("string"),
            "model": kind("string"), "mode": kind("string"), "aspectRatio": kind("string"),
            "duration": kind("number"), "width": kind("integer"), "height": kind("integer"),
            "price": amount_schema(), "verdict": kind("string"), "starred": kind("boolean"),
            "createdAt": kind("string"), "prompt": kind("string"),
            "promptTruncated": kind("boolean"), "versionId": kind("string"), "version": kind("integer"),
            "versionCount": kind("integer"), "jobId": kind("string"), "onHiggsfield": kind("boolean"),
        },
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "role": { "type": "string", "enum": raw_values(AgentInputRole::ALL, AgentInputRole::as_str) },
            "position": kind("integer"), "file": kind("string"), "item": kind("string"),
            "onHiggsfield": kind("boolean"), "start": kind("number"), "end": kind("number"),
        },
    })
}

fn settings_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "resolution": kind("string"), "draft": kind("boolean"), "audio": kind("string"),
            "style": kind("string"), "length": kind("number"), "aspectRatio": kind("string"),
            "paidWithUnlimited": kind("boolean"), "operation": kind("string"), "count": kind("integer"),
        },
    })
}

fn delivery_set_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": kind("string"), "name": kind("string"), "createdAt": kind("string"),
            "itemCount": kind("integer"), "frozen": kind("boolean"), "exportedAt": kind("string"),
            "folder": kind("string"), "outputs": list_of(kind("string")),
        },
    })
}

fn export_file_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "item": kind("string"), "status": { "type": "string", "enum": ["done", "failed", "skipped"] },
            "file": kind("string"), "sidecar": kind("string"), "message": kind("string"),
            "output": kind("string"),
        },
    })
}

pub(crate) fn request_id_schema() -> Value {
    json!({
        "type": "string", "pattern": "^[A-Za-z0-9_-][A-Za-z0-9._-]{0,63}$",
        "description": "Your own new ID for this request; the same one again only to retry it.",
    })
}

pub(crate) fn max_cost_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "description": "The most you agree to pay for this request, in the connection's unit.",
        "properties": {
            "amount": { "type": "number", "minimum": 0 },
            "unit": { "type": "string", "enum": ["usd", "credits"] },
        },
        "required": ["amount", "unit"],
    })
}

fn error_schema() -> Value {
    json!({
        "type": "object",
        "description": "Why the call failed: code to branch on, message for the person, next for what to do.",
        "properties": {
            "code": kind("string"), "message": kind("string"), "retryable": kind("boolean"),
            "next": kind("string"),
        },
        "required": ["code", "message", "retryable"],
    })
}
