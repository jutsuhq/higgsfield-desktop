//! `TemplateRecord`: saved `JobTemplate`s, by name.

use jutsu_core::JobTemplate;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;

use crate::error::StoreError;
use crate::store::{Store, StoreChange, nanos, text};

const TABLE: &str = "templates";

fn decode(row: &SqliteRow) -> Result<JobTemplate, StoreError> {
    let id = text(row, "id")?;
    serde_json::from_str(&text(row, "template_data")?).map_err(|error| StoreError::damaged(TABLE, id, error))
}

fn encode(template: &JobTemplate) -> Result<String, StoreError> {
    serde_json::to_string(template).map_err(StoreError::encode("template"))
}

impl Store {
    /// Every template, by name, then oldest first. A damaged entry is reported.
    pub async fn templates(&self) -> Result<Vec<JobTemplate>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, template_data FROM templates ORDER BY name COLLATE NOCASE ASC, created_at ASC",
        )
        .fetch_all(self.pool())
        .await?;
        rows.iter().map(decode).collect()
    }

    /// The template of `id`.
    pub async fn template(&self, id: Uuid) -> Result<Option<JobTemplate>, StoreError> {
        let row = sqlx::query("SELECT id, template_data FROM templates WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.pool())
            .await?;
        row.as_ref().map(decode).transpose()
    }

    /// The names of every template but `excluding`, by name, read from the name column.
    pub async fn template_names(&self, excluding: Option<Uuid>) -> Result<Vec<String>, StoreError> {
        Ok(sqlx::query_scalar(
            "SELECT name FROM templates WHERE ?1 IS NULL OR id != ?1 ORDER BY name COLLATE NOCASE ASC, created_at ASC",
        )
        .bind(excluding.map(|id| id.to_string()))
        .fetch_all(self.pool())
        .await?)
    }

    /// Adds `template`. Its ID must be new.
    pub async fn add_template(&self, template: &JobTemplate) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        sqlx::query("INSERT INTO templates (id, name, created_at, template_data) VALUES (?, ?, ?, ?)")
            .bind(template.id.to_string())
            .bind(&template.name)
            .bind(nanos(template.created_at))
            .bind(encode(template)?)
            .execute(&mut *write.transaction)
            .await?;
        write.commit(&[StoreChange::Templates]).await
    }

    /// Renames the template of `id`; the name column and the stored JSON change together. Nothing when it is gone.
    pub async fn rename_template(&self, id: Uuid, name: &str) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        let connection = &mut *write.transaction;
        let row = sqlx::query("SELECT id, template_data FROM templates WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *connection)
            .await?;
        let Some(row) = row else { return Ok(()) };
        let mut template = decode(&row)?;
        name.clone_into(&mut template.name);
        sqlx::query("UPDATE templates SET name = ?, template_data = ? WHERE id = ?")
            .bind(name)
            .bind(encode(&template)?)
            .bind(id.to_string())
            .execute(&mut *connection)
            .await?;
        write.commit(&[StoreChange::Templates]).await
    }

    /// Removes the template of `id`; nothing when it isn't stored. Its copied images are `FileLayout`'s.
    pub async fn delete_template(&self, id: Uuid) -> Result<(), StoreError> {
        let mut write = self.begin().await?;
        sqlx::query("DELETE FROM templates WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *write.transaction)
            .await?;
        write.commit(&[StoreChange::Templates]).await
    }
}
