//! `StarredPromptRecord` and the prompt history read from job records.

use sqlx::Row;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::StoreError;
use crate::store::{Store, StoreChange, date, nanos, text, uuid};

/// A starred prompt, pinned on top of the prompt history and kept when the history is cleared.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StarredPrompt {
    /// Trimmed, as the prompt history keys it.
    pub text: String,
    /// When it was starred.
    pub starred_at: OffsetDateTime,
}

/// A job's prompt, as the prompt history lists it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PromptUse {
    /// The job.
    pub job_id: Uuid,
    /// Its prompt.
    pub text: String,
    /// When the job was queued.
    pub created_at: OffsetDateTime,
}

impl Store {
    /// Every starred prompt, most recently starred first.
    pub async fn starred_prompts(&self) -> Result<Vec<StarredPrompt>, StoreError> {
        let rows = sqlx::query("SELECT text, starred_at FROM starred_prompts ORDER BY starred_at DESC")
            .fetch_all(self.pool())
            .await?;
        rows.iter()
            .map(|row| {
                let text = text(row, "text")?;
                let starred_at = date("starred_prompts", &text, row.try_get("starred_at")?)?;
                Ok(StarredPrompt { text, starred_at })
            })
            .collect()
    }

    /// Stars `text` (already keyed: trimmed) at `date`, or unstars it. Empty text and starring a starred prompt
    /// change nothing; a prompt is starred at most once.
    pub async fn set_prompt_starred(
        &self,
        text: &str,
        starred: bool,
        date: OffsetDateTime,
    ) -> Result<(), StoreError> {
        if text.is_empty() {
            return Ok(());
        }
        let mut write = self.begin().await?;
        if starred {
            sqlx::query(
                "INSERT INTO starred_prompts (text, starred_at) VALUES (?, ?) ON CONFLICT (text) DO NOTHING",
            )
            .bind(text)
            .bind(nanos(date))
            .execute(&mut *write.transaction)
            .await?;
        } else {
            sqlx::query("DELETE FROM starred_prompts WHERE text = ?")
                .bind(text)
                .execute(&mut *write.transaction)
                .await?;
        }
        write.commit(&[StoreChange::StarredPrompts]).await
    }

    /// Jobs with a prompt created after `after` (all of them for `None`), newest first: the prompt history.
    pub async fn prompt_history(&self, after: Option<OffsetDateTime>) -> Result<Vec<PromptUse>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, prompt, created_at FROM jobs WHERE created_at > ? AND prompt != '' ORDER BY created_at DESC",
        )
        .bind(after.map_or(i64::MIN, nanos))
        .fetch_all(self.pool())
        .await?;
        rows.iter()
            .map(|row| {
                let id = text(row, "id")?;
                Ok(PromptUse {
                    job_id: uuid("jobs", &id)?,
                    text: text(row, "prompt")?,
                    created_at: date("jobs", &id, row.try_get("created_at")?)?,
                })
            })
            .collect()
    }
}
