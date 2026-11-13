//! Read off the iam tables by value. Nothing here is written and no key
//! points at those tables, so the iam module stays free to change them.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::module::schema::core::domain::{Collaborator, DomainError};
use crate::module::schema::core::ports::UserDirectory;

use super::postgres::storage;

/// A deleted account cannot be shared with, and is not named either.
const SELECT_COLLABORATOR: &str = "SELECT u.id, u.email, u.first_name, u.last_name, u.avatar_url \
     FROM iam.users u \
     INNER JOIN iam.user_statuses s ON s.id = u.status_id AND s.status <> 'Deleted'";

pub struct PgUserDirectory {
    db: Arc<Database>,
}

impl PgUserDirectory {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn map(row: &PgRow) -> Result<Collaborator, DomainError> {
        let first_name: String = row.try_get("first_name").map_err(storage)?;
        let last_name: String = row.try_get("last_name").map_err(storage)?;

        Ok(Collaborator {
            id: row.try_get("id").map_err(storage)?,
            email: row.try_get("email").map_err(storage)?,
            name: format!("{} {}", first_name.trim(), last_name.trim())
                .trim()
                .to_owned(),
            avatar_url: row.try_get("avatar_url").map_err(storage)?,
        })
    }
}

#[async_trait]
impl UserDirectory for PgUserDirectory {
    async fn find_by_email(&self, email: &str) -> Result<Option<Collaborator>, DomainError> {
        let row = sqlx::query(&format!(
            "{SELECT_COLLABORATOR} WHERE LOWER(u.email) = LOWER($1)"
        ))
        .bind(email)
        .fetch_optional(self.db.pool())
        .await
        .map_err(storage)?;

        row.as_ref().map(Self::map).transpose()
    }

    async fn find_by_ids(&self, ids: &[Uuid]) -> Result<Vec<Collaborator>, DomainError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query(&format!("{SELECT_COLLABORATOR} WHERE u.id = ANY($1)"))
            .bind(ids)
            .fetch_all(self.db.pool())
            .await
            .map_err(storage)?;

        rows.iter().map(Self::map).collect()
    }
}
