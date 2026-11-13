// Several test modules share these and each uses a different handful of
// the helpers, so what one leaves alone would otherwise read as dead.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::module::schema::core::domain::{
    Access, Collaborator, DomainError, Grant, Invitation, Share, ShareRole, SharedSchema,
};
use crate::module::schema::core::ports::{
    InvitationDraft, SchemaRepository, ShareRepository, UserDirectory,
};
use crate::package::pagination::ListRequest;

#[derive(Default)]
pub(crate) struct FakeShares {
    owners: Mutex<HashMap<String, Uuid>>,
    shares: Mutex<Vec<Share>>,
    invitations: Mutex<Vec<(String, Invitation)>>,
    summaries: Mutex<HashMap<String, crate::module::schema::core::domain::SchemaSummary>>,
}

impl FakeShares {
    pub(crate) fn owned(self, schema_id: &str, owner: Uuid) -> Self {
        self.owners
            .lock()
            .unwrap()
            .insert(schema_id.to_owned(), owner);
        self
    }

    pub(crate) fn own(&self, schema_id: &str, owner: Uuid) {
        self.owners
            .lock()
            .unwrap()
            .insert(schema_id.to_owned(), owner);
    }

    /// The real store reads ownership and summaries off the schema tables.
    /// Here they are copied in from the schema fake.
    pub(crate) async fn mirror(&self, owner: Uuid, schemas: &dyn SchemaRepository) {
        let (summaries, _) = schemas
            .index(owner, &ListRequest::paged(1, 100))
            .await
            .expect("the listing should succeed");

        for summary in summaries {
            self.own(&summary.id, owner);
            self.summaries
                .lock()
                .unwrap()
                .insert(summary.id.clone(), summary);
        }
    }

    pub(crate) fn share_of(&self, schema_id: &str, user_id: Uuid) -> Option<Share> {
        self.shares
            .lock()
            .unwrap()
            .iter()
            .find(|share| share.schema_id == schema_id && share.user_id == user_id)
            .cloned()
    }

    pub(crate) fn invitations_for(&self, email: &str) -> Vec<Invitation> {
        self.invitations
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, invitation)| invitation.email == email)
            .map(|(_, invitation)| invitation.clone())
            .collect()
    }

    pub(crate) fn token_of(&self, invitation_id: Uuid) -> Option<String> {
        self.invitations
            .lock()
            .unwrap()
            .iter()
            .find(|(_, invitation)| invitation.id == invitation_id)
            .map(|(token, _)| token.clone())
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("a fixed instant")
    }

    fn write(&self, schema_id: &str, user_id: Uuid, role: ShareRole, shared_by: Uuid) -> Share {
        let mut shares = self.shares.lock().unwrap();

        if let Some(share) = shares
            .iter_mut()
            .find(|share| share.schema_id == schema_id && share.user_id == user_id)
        {
            share.role = role;
            return share.clone();
        }

        let share = Share {
            schema_id: schema_id.to_owned(),
            user_id,
            role,
            shared_by,
            created_at: Self::now(),
            updated_at: Self::now(),
        };
        shares.push(share.clone());

        share
    }

    fn take_invitation(&self, id: Uuid) -> Option<Invitation> {
        let mut invitations = self.invitations.lock().unwrap();
        let index = invitations
            .iter()
            .position(|(_, invitation)| invitation.id == id)?;

        Some(invitations.remove(index).1)
    }
}

#[async_trait]
impl ShareRepository for FakeShares {
    async fn grant(&self, caller: Uuid, schema_id: &str) -> Result<Option<Grant>, DomainError> {
        let Some(owner) = self.owners.lock().unwrap().get(schema_id).copied() else {
            return Ok(None);
        };

        if owner == caller {
            return Ok(Some(Grant {
                owner,
                access: Access::Owner,
            }));
        }

        Ok(self.share_of(schema_id, caller).map(|share| Grant {
            owner,
            access: Access::Shared(share.role),
        }))
    }

    async fn shared_with(
        &self,
        caller: Uuid,
        request: &ListRequest,
    ) -> Result<(Vec<SharedSchema>, i64), DomainError> {
        let owners = self.owners.lock().unwrap();
        let summaries = self.summaries.lock().unwrap();
        let mine: Vec<SharedSchema> = self
            .shares
            .lock()
            .unwrap()
            .iter()
            .filter(|share| share.user_id == caller)
            .filter_map(|share| {
                Some(SharedSchema {
                    summary: summaries.get(&share.schema_id)?.clone(),
                    role: share.role,
                    owner_id: *owners.get(&share.schema_id)?,
                })
            })
            .collect();
        let total = mine.len() as i64;

        Ok((
            mine.into_iter()
                .skip(request.offset() as usize)
                .take(request.per_page as usize)
                .collect(),
            total,
        ))
    }

    async fn shares(&self, schema_id: &str) -> Result<Vec<Share>, DomainError> {
        Ok(self
            .shares
            .lock()
            .unwrap()
            .iter()
            .filter(|share| share.schema_id == schema_id)
            .cloned()
            .collect())
    }

    async fn invitations(&self, schema_id: &str) -> Result<Vec<Invitation>, DomainError> {
        Ok(self
            .invitations
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, invitation)| invitation.schema_id == schema_id)
            .map(|(_, invitation)| invitation.clone())
            .collect())
    }

    async fn create_share(
        &self,
        schema_id: &str,
        user_id: Uuid,
        role: ShareRole,
        shared_by: Uuid,
    ) -> Result<Share, DomainError> {
        if self.share_of(schema_id, user_id).is_some() {
            return Err(DomainError::AlreadyShared(user_id.to_string()));
        }

        Ok(self.write(schema_id, user_id, role, shared_by))
    }

    async fn update_share(
        &self,
        schema_id: &str,
        user_id: Uuid,
        role: ShareRole,
    ) -> Result<Share, DomainError> {
        let shared_by = self
            .share_of(schema_id, user_id)
            .ok_or(DomainError::ShareNotFound)?
            .shared_by;

        Ok(self.write(schema_id, user_id, role, shared_by))
    }

    async fn delete_share(&self, schema_id: &str, user_id: Uuid) -> Result<(), DomainError> {
        let mut shares = self.shares.lock().unwrap();
        let before = shares.len();
        shares.retain(|share| !(share.schema_id == schema_id && share.user_id == user_id));

        if shares.len() == before {
            return Err(DomainError::ShareNotFound);
        }

        Ok(())
    }

    async fn create_invitation(&self, draft: InvitationDraft) -> Result<Invitation, DomainError> {
        let mut invitations = self.invitations.lock().unwrap();

        if invitations.iter().any(|(_, invitation)| {
            invitation.schema_id == draft.schema_id && invitation.email == draft.email
        }) {
            return Err(DomainError::AlreadyInvited(draft.email));
        }

        let invitation = Invitation {
            id: Uuid::new_v4(),
            schema_id: draft.schema_id,
            email: draft.email,
            role: draft.role,
            invited_by: draft.invited_by,
            expires_at: draft.expires_at,
            created_at: Self::now(),
        };
        invitations.push((draft.token_hash, invitation.clone()));

        Ok(invitation)
    }

    async fn delete_invitation(
        &self,
        schema_id: &str,
        invitation_id: Uuid,
    ) -> Result<(), DomainError> {
        let mut invitations = self.invitations.lock().unwrap();
        let before = invitations.len();
        invitations.retain(|(_, invitation)| {
            !(invitation.schema_id == schema_id && invitation.id == invitation_id)
        });

        if invitations.len() == before {
            return Err(DomainError::InvitationNotFound);
        }

        Ok(())
    }

    async fn find_invitation_by_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<Invitation>, DomainError> {
        Ok(self
            .invitations
            .lock()
            .unwrap()
            .iter()
            .find(|(token, _)| token == token_hash)
            .map(|(_, invitation)| invitation.clone()))
    }

    async fn accept_invitation(
        &self,
        invitation_id: Uuid,
        user_id: Uuid,
    ) -> Result<Share, DomainError> {
        let invitation = self
            .take_invitation(invitation_id)
            .ok_or(DomainError::InvitationNotFound)?;

        Ok(self.write(
            &invitation.schema_id,
            user_id,
            invitation.role,
            invitation.invited_by,
        ))
    }

    async fn claim_invitations(
        &self,
        email: &str,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Vec<Share>, DomainError> {
        let waiting: Vec<Invitation> = self
            .invitations_for(email)
            .into_iter()
            .filter_map(|invitation| self.take_invitation(invitation.id))
            .collect();

        Ok(waiting
            .into_iter()
            .filter(|invitation| !invitation.is_expired_at(now))
            .map(|invitation| {
                self.write(
                    &invitation.schema_id,
                    user_id,
                    invitation.role,
                    invitation.invited_by,
                )
            })
            .collect())
    }
}

#[derive(Default)]
pub(crate) struct FakeDirectory {
    people: Vec<Collaborator>,
}

impl FakeDirectory {
    pub(crate) fn knowing(people: Vec<Collaborator>) -> Self {
        Self { people }
    }
}

#[async_trait]
impl UserDirectory for FakeDirectory {
    async fn find_by_email(&self, email: &str) -> Result<Option<Collaborator>, DomainError> {
        Ok(self
            .people
            .iter()
            .find(|person| person.email.eq_ignore_ascii_case(email))
            .cloned())
    }

    async fn find_by_ids(&self, ids: &[Uuid]) -> Result<Vec<Collaborator>, DomainError> {
        Ok(self
            .people
            .iter()
            .filter(|person| ids.contains(&person.id))
            .cloned()
            .collect())
    }
}
