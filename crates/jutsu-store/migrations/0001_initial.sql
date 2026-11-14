-- Each record keeps its whole value as JSON (`*_data`) plus the columns queries filter and sort on.
-- Dates are nanoseconds since 1970 (UTC); booleans are 0 or 1.

-- `JobRecord`: the execution and spending ledger.
CREATE TABLE jobs (
    id TEXT PRIMARY KEY NOT NULL,
    created_at INTEGER NOT NULL,
    finished_at INTEGER,
    -- `JobState` case name without its payload, e.g. "done" or "failed".
    state_kind TEXT NOT NULL,
    is_terminal INTEGER NOT NULL,
    is_draft INTEGER NOT NULL,
    mode TEXT NOT NULL,
    resolution TEXT NOT NULL,
    provider_kind TEXT NOT NULL,
    prompt TEXT NOT NULL,
    output_path TEXT,
    job_data TEXT NOT NULL,
    -- The input versions pinned when first enqueued, independently of later execution updates.
    input_references TEXT NOT NULL,
    remote_key TEXT
) STRICT;

CREATE INDEX jobs_terminal_created ON jobs (is_terminal, created_at);
CREATE INDEX jobs_terminal_finished ON jobs (is_terminal, finished_at);
CREATE INDEX jobs_state_created ON jobs (state_kind, created_at);
CREATE INDEX jobs_created ON jobs (created_at);
CREATE INDEX jobs_remote_key ON jobs (remote_key);

-- `MaterialRecord`: the Library catalog. `is_starred` is the star; the one inside `material_data` is not read.
CREATE TABLE materials (
    id TEXT PRIMARY KEY NOT NULL,
    created_at INTEGER NOT NULL,
    remote_key TEXT,
    -- The current version's file URL, as text.
    file_path TEXT,
    variant_group_id TEXT,
    kind TEXT NOT NULL,
    is_hidden INTEGER NOT NULL,
    -- The current version's culling.
    culling TEXT NOT NULL,
    material_data TEXT NOT NULL,
    is_starred INTEGER NOT NULL
) STRICT;

CREATE INDEX materials_created ON materials (created_at);
CREATE INDEX materials_kind_created ON materials (kind, created_at);
CREATE INDEX materials_remote_key ON materials (remote_key);
CREATE INDEX materials_file_path ON materials (file_path);
CREATE INDEX materials_variant_group ON materials (variant_group_id);

-- Every version's inputs, copied from `material_data` when the material is written, so "Used in" is one lookup.
CREATE TABLE material_inputs (
    material_id TEXT NOT NULL REFERENCES materials (id) ON DELETE CASCADE,
    version_id TEXT NOT NULL,
    -- The version's place among the material's versions, oldest first.
    version_index INTEGER NOT NULL,
    source_material_id TEXT NOT NULL,
    source_version_id TEXT NOT NULL,
    role TEXT NOT NULL,
    position INTEGER NOT NULL
) STRICT;

CREATE INDEX material_inputs_material ON material_inputs (material_id);
CREATE INDEX material_inputs_source ON material_inputs (source_material_id, source_version_id);

-- `AssetRecord`: an uploaded input, by the SHA-256 of its prepared file.
CREATE TABLE assets (
    content_hash TEXT PRIMARY KEY NOT NULL,
    remote_url TEXT NOT NULL,
    remote_id TEXT,
    uploaded_at INTEGER NOT NULL
) STRICT;

CREATE INDEX assets_uploaded ON assets (uploaded_at);

-- `StarredPromptRecord`: kept when the prompt history is cleared.
CREATE TABLE starred_prompts (
    text TEXT PRIMARY KEY NOT NULL,
    starred_at INTEGER NOT NULL
) STRICT;

CREATE INDEX starred_prompts_starred ON starred_prompts (starred_at);

-- `TemplateRecord`.
CREATE TABLE templates (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    template_data TEXT NOT NULL
) STRICT;

CREATE INDEX templates_name ON templates (name COLLATE NOCASE, created_at);

-- `DeliverySetRecord`.
CREATE TABLE delivery_sets (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    set_data TEXT NOT NULL
) STRICT;

CREATE INDEX delivery_sets_created ON delivery_sets (created_at);
