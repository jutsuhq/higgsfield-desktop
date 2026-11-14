//! Which files a delivery run writes, and their names.

use std::collections::HashSet;

use crate::models::{
    CropFrame, DeliveryItem, DeliveryOutput, DeliveryResult, DeliveryResultStatus, DeliverySet,
};
use crate::rules::{ExportNaming, OutputNamePattern};
use crate::support::url_ext::deleting_path_extension;
use crate::support::uuid_string;

/// One file of a delivery: an item in an output, under the name it gets.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryTask {
    /// The item.
    pub item: DeliveryItem,
    /// The output.
    pub output: DeliveryOutput,
    /// The file's name.
    pub file_name: String,
    /// The crop it is written with; `None` for an output that doesn't crop.
    pub frame: Option<CropFrame>,
    /// The set wrote this name for the same item and output before, so the new file takes its place. Any other
    /// existing file is never replaced.
    pub replaces_existing: bool,
}

impl DeliveryTask {
    /// "<version ID>/<output ID>".
    pub fn id(&self) -> String {
        format!(
            "{}/{}",
            uuid_string(self.item.reference.version_id),
            uuid_string(self.output.id)
        )
    }

    /// The task's result; the file name only when it is done.
    pub fn result(&self, status: DeliveryResultStatus, message: Option<String>) -> DeliveryResult {
        let mut result = DeliveryResult::new(self.item.reference, self.output.clone(), status);
        result.file_name = (status == DeliveryResultStatus::Done).then(|| self.file_name.clone());
        result.frame = self.frame;
        result.message = message;
        result
    }
}

/// Which files a delivery run writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeliveryScope {
    /// Every item in every output.
    All,
    /// What isn't exported as the set is now: never exported, failed or skipped, or exported with another crop
    /// or preset. After fixing one item's crop, only that item's files.
    Pending,
    /// Only the files whose last run failed.
    Failed,
}

/// Which files a delivery run writes, and their names (`ExportNaming::delivery_base_name`). Image outputs take the
/// set's images and video outputs its videos.
pub struct DeliveryPlan;

impl DeliveryPlan {
    /// The files of `set` in `scope`. `is_taken` says whether a file of that name is already in the folder; compare
    /// case-insensitively on the usual case-insensitive volumes.
    pub fn tasks(
        set: &DeliverySet,
        scope: DeliveryScope,
        is_taken: impl Fn(&str) -> bool,
    ) -> Vec<DeliveryTask> {
        let labels = ExportNaming::output_labels(&set.outputs);
        let mut planned: HashSet<String> = HashSet::new();
        let mut tasks = Vec::new();
        for (offset, item) in set.items().iter().enumerate() {
            for output in set.outputs.iter().filter(|output| output.kind() == item.kind) {
                let previous = set.result(item, output);
                let frame = item.frame(output.aspect);
                if !Self::includes(previous, output, frame, scope) {
                    continue;
                }
                let base = ExportNaming::delivery_base_name(
                    &set.name,
                    offset + 1,
                    set.items().len(),
                    &item.name,
                    &labels
                        .get(&output.id)
                        .cloned()
                        .unwrap_or_else(|| output.summary()),
                );
                // A file this set wrote for the same item and output is written again under its name, with subtitle
                // files beside it only where it wrote them too, or where no other file has their names.
                let own_file = previous
                    .filter(|previous| previous.status == DeliveryResultStatus::Done)
                    .and_then(|previous| previous.file_name.clone().map(|name| (previous, name)))
                    .and_then(|(previous, name)| {
                        let names = Self::names(&name, output);
                        let owns_sides = previous.output.subtitle_formats() == output.subtitle_formats()
                            || !names[1..]
                                .iter()
                                .any(|side| is_taken(side) || planned.contains(&side.to_lowercase()));
                        (name.ends_with(&format!(".{}", output.file_extension()))
                            && !planned.contains(&name.to_lowercase())
                            && owns_sides)
                            .then_some(name)
                    });
                let replaces_existing = own_file.is_some();
                // Subtitle files go next to the video under its name, so their names have to be free too.
                let name = own_file.unwrap_or_else(|| {
                    OutputNamePattern::available(
                        &format!("{base}.{}", output.file_extension()),
                        |candidate| {
                            Self::names(candidate, output)
                                .iter()
                                .any(|name| planned.contains(&name.to_lowercase()) || is_taken(name))
                        },
                    )
                });
                for taken in Self::names(&name, output) {
                    planned.insert(taken.to_lowercase());
                }
                tasks.push(DeliveryTask {
                    item: item.clone(),
                    output: output.clone(),
                    file_name: name,
                    frame,
                    replaces_existing,
                });
            }
        }
        tasks
    }

    /// The video's file name and those of the subtitle files next to it.
    pub fn names(file_name: &str, output: &DeliveryOutput) -> Vec<String> {
        let stem = deleting_path_extension(file_name);
        std::iter::once(file_name.to_owned())
            .chain(
                output
                    .subtitle_formats()
                    .into_iter()
                    .map(|format| format!("{stem}.{}", format.file_extension())),
            )
            .collect()
    }

    fn includes(
        previous: Option<&DeliveryResult>,
        output: &DeliveryOutput,
        frame: Option<CropFrame>,
        scope: DeliveryScope,
    ) -> bool {
        match scope {
            DeliveryScope::All => true,
            DeliveryScope::Failed => {
                previous.is_some_and(|previous| previous.status == DeliveryResultStatus::Failed)
            }
            DeliveryScope::Pending => match previous {
                Some(previous) if previous.status == DeliveryResultStatus::Done => {
                    previous.output != *output || previous.frame != frame
                }
                _ => true,
            },
        }
    }
}
