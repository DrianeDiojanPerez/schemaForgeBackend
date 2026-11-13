use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::database::{pg_error, Database};
use crate::module::schema::core::domain::{
    Access, DomainError, Grant, Invitation, Share, ShareRole, SharedSchema,
};
use crate::module::schema::core::ports::{InvitationDraft, ShareRepository};
use crate::package::pagination::ListRequest;

use super::postgres::{map_summary, parse_id, storage};

const SELECT_SHARE: &str = "SELECT schema_id, user_id, role, shared_by, created_at, updated_at \
     FROM schemaforge.schema_shares";

const RETURNING_SHARE: &str =
    "RETURNING schema_id, user_id, role, shared_by, created_at, updated_at";

const SELECT_INVITATION: &str =
    "SELECT id, schema_id, email, role, invited_by, expires_at, created_at \
     FROM schemaforge.schema_invitations";

const RETURNING_INVITATION: &str =
    "RETURNING id, schema_id, email, role, invited_by, expires_at, created_at";

/// Accepting twice, or being shared with directly in between, still ends
/// with one row.
const UPSERT_SHARE: &str =
    "INSERT INTO schemaforge.schema_shares (schema_id, user_id, role, shared_by) \
     VALUES ($1, $2, $3, $4) \
     ON CONFLICT (schema_id, user_id) DO UPDATE SET role = EXCLUDED.role, updated_at = NOW()";

pub struct PgShareRepository {
    db: Arc<Database>,
}

impl PgShareRepository {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn role(row: &PgRow, column: &str) -> Result<ShareRole, DomainError> {
        let role: String = row.try_get(column).map_err(storage)?;

        role.parse()
            .map_err(|()| DomainError::Storage(format!("`{role}` is not a known share role")))
    }

    fn map_share(row: &PgRow) -> Result<Share, DomainError> {
        let schema_id: Uuid = row.try_get("schema_id").map_err(storage)?;

        Ok(Share {
            schema_id: schema_id.to_string(),
            user_id: row.try_get("user_id").map_err(storage)?,
            role: Self::role(row, "role")?,
            shared_by: row.try_get("shared_by").map_err(storage)?,
            created_at: row.try_get("created_at").map_err(storage)?,
            updated_at: row.try_get("updated_at").map_err(storage)?,
        })
    }

    fn map_invitation(row: &PgRow) -> Result<Invitation, DomainError> {
        let schema_id: Uuid = row.try_get("schema_id").map_err(storage)?;

        Ok(Invitation {
            id: row.try_get("id").map_err(storage)?,
            schema_id: schema_id.to_string(),
            email: row.try_get("email").map_err(storage)?,
            role: Self::role(row, "role")?,
            invited_by: row.try_get("invited_by").map_err(storage)?,
            expires_at: row.try_get("expires_at").map_err(storage)?,
            created_at: row.try_get("created_at").map_err(storage)?,
        })
    }

    async fn upsert_share(
        conn: &mut PgConnection,
        schema_id: Uuid,
        user_id: Uuid,
        role: ShareRole,
        shared_by: Uuid,
    ) -> Result<Share, DomainError> {
        let row = sqlx::query(&format!("{UPSERT_SHARE} {RETURNING_SHARE}"))
            .bind(schema_id)
            .bind(user_id)
            .bind(role.as_str())
            .bind(shared_by)
            .fetch_one(conn)
            .await
            .map_err(storage)?;

        Self::map_share(&row)
    }

    /// Consumed and written in one transaction, so a link can never grant
    /// access twice or leave the invitation behind.
    async fn convert(
        conn: &mut PgConnection,
        invitation: &Invitation,
        user_id: Uuid,
    ) -> Result<Share, DomainError> {
        let schema_id = parse_id(&invitation.schema_id).ok_or(DomainError::InvitationNotFound)?;

        sqlx::query("DELETE FROM schemaforge.schema_invitations WHERE id = $1")
            .bind(invitation.id)
            .execute(&mut *conn)
            .await
            .map_err(storage)?;

        Self::upsert_share(
            conn,
            schema_id,
            user_id,
            invitation.role,
            invitation.invited_by,
        )
        .await
    }
}

#[async_trait]
impl ShareRepository for PgShareRepository {
    async fn grant(&self, caller: Uuid, schema_id: &str) -> Result<Option<Grant>, DomainError> {
        let Some(uuid) = parse_id(schema_id) else {
            return Ok(None);
        };

        let row = sqlx::query(
            "SELECT us.user_id AS owner, ss.role \
             FROM schemaforge.user_schemas us \
             LEFT JOIN schemaforge.schema_shares ss \
                 ON ss.schema_id = us.schema_id AND ss.user_id = $1 \
             WHERE us.schema_id = $2 AND (us.user_id = $1 OR ss.user_id IS NOT NULL)",
        )
        .bind(caller)
        .bind(uuid)
        .fetch_optional(self.db.pool())
        .await
        .map_err(storage)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let owner: Uuid = row.try_get("owner").map_err(storage)?;
        let access = if owner == caller {
            Access::Owner
        } else {
            Access::Shared(Self::role(&row, "role")?)
        };

        Ok(Some(Grant { owner, access }))
    }

    async fn shared_with(
        &self,
        caller: Uuid,
        request: &ListRequest,
    ) -> Result<(Vec<SharedSchema>, i64), DomainError> {
        let total: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM schemaforge.schema_shares WHERE user_id = $1")
                .bind(caller)
                .fetch_one(self.db.pool())
                .await
                .map_err(storage)?;

        let rows = sqlx::query(
            "SELECT s.id, s.name, s.description, \
                 jsonb_array_length(s.entities) AS entity_count, \
                 jsonb_array_length(s.relationships) AS relationship_count, \
                 s.created_at, s.updated_at, ss.role, us.user_id AS owner_id \
             FROM schemaforge.schema_shares ss \
             INNER JOIN schemaforge.schemas s ON s.id = ss.schema_id \
             INNER JOIN schemaforge.user_schemas us ON us.schema_id = s.id \
             WHERE ss.user_id = $1 \
             ORDER BY ss.created_at, ss.schema_id LIMIT $2 OFFSET $3",
        )
        .bind(caller)
        .bind(request.per_page)
        .bind(request.offset())
        .fetch_all(self.db.pool())
        .await
        .map_err(storage)?;

        let shared = rows
            .iter()
            .map(|row| {
                Ok(SharedSchema {
                    summary: map_summary(row)?,
                    role: Self::role(row, "role")?,
                    owner_id: row.try_get("owner_id").map_err(storage)?,
                })
            })
            .collect::<Result<Vec<_>, DomainError>>()?;

        Ok((shared, total))
    }

    async fn shares(&self, schema_id: &str) -> Result<Vec<Share>, DomainError> {
        let Some(uuid) = parse_id(schema_id) else {
            return Ok(Vec::new());
        };

        let rows = sqlx::query(&format!(
            "{SELECT_SHARE} WHERE schema_id = $1 ORDER BY created_at, user_id"
        ))
        .bind(uuid)
        .fetch_all(self.db.pool())
        .await
        .map_err(storage)?;

        rows.iter().map(Self::map_share).collect()
    }

    async fn invitations(&self, schema_id: &str) -> Result<Vec<Invitation>, DomainError> {
        let Some(uuid) = parse_id(schema_id) else {
            return Ok(Vec::new());
        };

        let rows = sqlx::query(&format!(
            "{SELECT_INVITATION} WHERE schema_id = $1 ORDER BY created_at, id"
        ))
        .bind(uuid)
        .fetch_all(self.db.pool())
        .await
        .map_err(storage)?;

        rows.iter().map(Self::map_invitation).collect()
    }

    async fn create_share(
        &self,
        schema_id: &str,
        user_id: Uuid,
        role: ShareRole,
        shared_by: Uuid,
    ) -> Result<Share, DomainError> {
        let uuid = parse_id(schema_id).ok_or(DomainError::SchemaNotFound)?;

        let row = sqlx::query(&format!(
            "INSERT INTO schemaforge.schema_shares (schema_id, user_id, role, shared_by) \
             VALUES ($1, $2, $3, $4) {RETURNING_SHARE}"
        ))
        .bind(uuid)
        .bind(user_id)
        .bind(role.as_str())
        .bind(shared_by)
        .fetch_one(self.db.pool())
        .await
        .map_err(|err| {
            if pg_error::is(&err, pg_error::UNIQUE_VIOLATION) {
                return DomainError::AlreadyShared(user_id.to_string());
            }

            storage(err)
        })?;

        Self::map_share(&row)
    }

    async fn update_share(
        &self,
        schema_id: &str,
        user_id: Uuid,
        role: ShareRole,
    ) -> Result<Share, DomainError> {
        let uuid = parse_id(schema_id).ok_or(DomainError::ShareNotFound)?;

        let row = sqlx::query(&format!(
            "UPDATE schemaforge.schema_shares SET role = $3, updated_at = NOW() \
             WHERE schema_id = $1 AND user_id = $2 {RETURNING_SHARE}"
        ))
        .bind(uuid)
        .bind(user_id)
        .bind(role.as_str())
        .fetch_optional(self.db.pool())
        .await
        .map_err(storage)?
        .ok_or(DomainError::ShareNotFound)?;

        Self::map_share(&row)
    }

    async fn delete_share(&self, schema_id: &str, user_id: Uuid) -> Result<(), DomainError> {
        let uuid = parse_id(schema_id).ok_or(DomainError::ShareNotFound)?;

        let result = sqlx::query(
            "DELETE FROM schemaforge.schema_shares WHERE schema_id = $1 AND user_id = $2",
        )
        .bind(uuid)
        .bind(user_id)
        .execute(self.db.pool())
        .await
        .map_err(storage)?;

        if result.rows_affected() == 0 {
            return Err(DomainError::ShareNotFound);
        }

        Ok(())
    }

    async fn create_invitation(&self, draft: InvitationDraft) -> Result<Invitation, DomainError> {
        let uuid = parse_id(&draft.schema_id).ok_or(DomainError::SchemaNotFound)?;

        let row = sqlx::query(&format!(
            "INSERT INTO schemaforge.schema_invitations \
                 (schema_id, email, role, token, invited_by, expires_at) \
             VALUES ($1, $2, $3, $4, $5, $6) {RETURNING_INVITATION}"
        ))
        .bind(uuid)
        .bind(&draft.email)
        .bind(draft.role.as_str())
        .bind(&draft.token_hash)
        .bind(draft.invited_by)
        .bind(draft.expires_at)
        .fetch_one(self.db.pool())
        .await
        .map_err(|err| {
            if pg_error::is(&err, pg_error::UNIQUE_VIOLATION) {
                return DomainError::AlreadyInvited(draft.email.clone());
            }

            storage(err)
        })?;

        Self::map_invitation(&row)
    }

    async fn delete_invitation(
        &self,
        schema_id: &str,
        invitation_id: Uuid,
    ) -> Result<(), DomainError> {
        let uuid = parse_id(schema_id).ok_or(DomainError::InvitationNotFound)?;

        let result = sqlx::query(
            "DELETE FROM schemaforge.schema_invitations WHERE schema_id = $1 AND id = $2",
        )
        .bind(uuid)
        .bind(invitation_id)
        .execute(self.db.pool())
        .await
        .map_err(storage)?;

        if result.rows_affected() == 0 {
            return Err(DomainError::InvitationNotFound);
        }

        Ok(())
    }

    async fn find_invitation_by_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<Invitation>, DomainError> {
        let row = sqlx::query(&format!("{SELECT_INVITATION} WHERE token = $1"))
            .bind(token_hash)
            .fetch_optional(self.db.pool())
            .await
            .map_err(storage)?;

        row.as_ref().map(Self::map_invitation).transpose()
    }

    async fn accept_invitation(
        &self,
        invitation_id: Uuid,
        user_id: Uuid,
    ) -> Result<Share, DomainError> {
        let mut tx = self.db.pool().begin().await.map_err(storage)?;

        let row = sqlx::query(&format!("{SELECT_INVITATION} WHERE id = $1 FOR UPDATE"))
            .bind(invitation_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(storage)?
            .ok_or(DomainError::InvitationNotFound)?;

        let share = Self::convert(&mut tx, &Self::map_invitation(&row)?, user_id).await?;

        tx.commit().await.map_err(storage)?;

        Ok(share)
    }

    async fn claim_invitations(
        &self,
        email: &str,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Vec<Share>, DomainError> {
        let mut tx = self.db.pool().begin().await.map_err(storage)?;

        let rows = sqlx::query(&format!(
            "{SELECT_INVITATION} WHERE LOWER(email) = LOWER($1) ORDER BY created_at FOR UPDATE"
        ))
        .bind(email)
        .fetch_all(&mut *tx)
        .await
        .map_err(storage)?;

        let mut shares = Vec::new();

        // An expired one is dropped rather than honoured, so the address is
        // free to be invited again.
        for row in &rows {
            let invitation = Self::map_invitation(row)?;

            if invitation.is_expired_at(now) {
                sqlx::query("DELETE FROM schemaforge.schema_invitations WHERE id = $1")
                    .bind(invitation.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(storage)?;

                continue;
            }

            shares.push(Self::convert(&mut tx, &invitation, user_id).await?);
        }

        tx.commit().await.map_err(storage)?;

        Ok(shares)
    }
}
