use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use uuid::Uuid;
use validator::ValidateEmail;

use crate::module::schema::core::domain::{
    Collaborator, DomainError, Grant, Invitation, Share, ShareRole, SharedSchema,
};
use crate::module::schema::core::ports::{
    InvitationDraft, Member, People, SchemaRepository, ShareRepository, ShareRequest, Shared,
    SharedListing, SharingService, UserDirectory,
};
use crate::package::crypto;
use crate::package::emailer::Emailer;
use crate::package::errdef::Error;
use crate::package::pagination::{Data, ListRequest};

const INVITATION_TTL_DAYS: i64 = 7;
const INVITATION_TEMPLATE: &str = "schema-invitation";
const SHARED_TEMPLATE: &str = "schema-shared";

pub struct SharingServiceImpl {
    schemas: Arc<dyn SchemaRepository>,
    shares: Arc<dyn ShareRepository>,
    directory: Arc<dyn UserDirectory>,
    mailer: Arc<dyn Emailer>,
}

fn to_error(failure: DomainError) -> Error {
    match failure {
        DomainError::SchemaNotFound => Error::not_found("schema does not exist"),
        DomainError::AlreadyShared(who) => {
            Error::conflict(format!("`{who}` already has access to this schema"))
        }
        DomainError::AlreadyInvited(email) => {
            Error::conflict(format!("`{email}` has already been invited to this schema"))
        }
        DomainError::ShareNotFound => Error::not_found("share does not exist"),
        DomainError::InvitationNotFound => Error::not_found("invitation does not exist"),
        other => Error::unknown(other),
    }
}

fn clean_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// The app's front door, taken off the invitation link the caller gave:
/// `https://app.example/invitations/` is the app at `https://app.example/`.
fn app_uri(callback_uri: &str) -> String {
    let trimmed = callback_uri.trim();
    let after_scheme = trimmed.find("://").map(|at| at + 3).unwrap_or(0);

    match trimmed[after_scheme..].find('/') {
        Some(slash) => format!("{}/", &trimmed[..after_scheme + slash]),
        None => format!("{trimmed}/"),
    }
}

/// What goes in the round badge at the top of the mail: the person's picture
/// when the account has one, and their initials otherwise. The template is
/// plain substitution, so the choice is made here.
fn avatar(picture: Option<&str>, name: &str) -> String {
    match picture.map(str::trim).filter(|url| url.starts_with("https://")) {
        Some(url) => format!(
            "<img src=\"{}\" width=\"44\" height=\"44\" alt=\"\" style=\"display: block; width: 44px; height: 44px; border-radius: 22px;\">",
            url.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
        ),
        None => initials(name),
    }
}

/// The first letter of the first two words, or of the address when that is
/// all there is to go on.
fn initials(name: &str) -> String {
    name.split([' ', '@'])
        .filter(|word| !word.is_empty())
        .take(2)
        .filter_map(|word| word.chars().next())
        .flat_map(char::to_uppercase)
        .collect()
}

/// Someone the directory no longer names still holds what the tables say,
/// so they are shown by id rather than dropped.
fn unnamed(id: Uuid) -> Collaborator {
    Collaborator {
        id,
        email: String::new(),
        name: String::new(),
        avatar_url: None,
    }
}

impl SharingServiceImpl {
    pub fn new(
        schemas: Arc<dyn SchemaRepository>,
        shares: Arc<dyn ShareRepository>,
        directory: Arc<dyn UserDirectory>,
        mailer: Arc<dyn Emailer>,
    ) -> Self {
        Self {
            schemas,
            shares,
            directory,
            mailer,
        }
    }

    /// A schema the caller has no part in reads as missing, so sharing
    /// cannot be used to probe for one.
    async fn grant(&self, caller: Uuid, schema_id: &str) -> Result<Grant, Error> {
        self.shares
            .grant(caller, schema_id)
            .await
            .map_err(to_error)?
            .ok_or_else(|| Error::not_found("schema does not exist"))
    }

    async fn owned(&self, caller: Uuid, schema_id: &str) -> Result<Grant, Error> {
        let grant = self.grant(caller, schema_id).await?;

        if !grant.access.is_owner() {
            return Err(Error::forbidden("only the owner may share this schema"));
        }

        Ok(grant)
    }

    /// How the person doing the sharing is named in a mail, and their picture.
    async fn sender(&self, caller: Uuid) -> Result<(String, Option<String>), Error> {
        let person = self.people_by_id(vec![caller]).await?.remove(&caller);
        let picture = person.as_ref().and_then(|person| person.avatar_url.clone());
        let name = person
            .map(|person| {
                if person.name.is_empty() {
                    person.email
                } else {
                    person.name
                }
            })
            .unwrap_or_else(|| "Someone".to_owned());

        Ok((name, picture))
    }

    async fn people_by_id(&self, ids: Vec<Uuid>) -> Result<HashMap<Uuid, Collaborator>, Error> {
        let mut unique = ids;
        unique.sort_unstable();
        unique.dedup();

        Ok(self
            .directory
            .find_by_ids(&unique)
            .await
            .map_err(to_error)?
            .into_iter()
            .map(|person| (person.id, person))
            .collect())
    }

    async fn member(&self, share: Share) -> Result<Member, Error> {
        let user = self
            .people_by_id(vec![share.user_id])
            .await?
            .remove(&share.user_id)
            .unwrap_or_else(|| unnamed(share.user_id));

        Ok(Member { share, user })
    }

    async fn listing_of(&self, caller: Uuid, shared: SharedSchema) -> Result<SharedListing, Error> {
        let owner = self
            .people_by_id(vec![shared.owner_id])
            .await?
            .remove(&shared.owner_id)
            .unwrap_or_else(|| unnamed(shared.owner_id));

        tracing::debug!(%caller, schema.id = %shared.summary.id, "shared schema read");

        Ok(SharedListing {
            summary: shared.summary,
            role: shared.role,
            owner,
        })
    }

    fn check_share(request: &ShareRequest) -> Result<(), Error> {
        let mut error = Error::validation("failed payload validation");

        if request.email.trim().is_empty() {
            error.push_violation("email", "field is required and cannot be empty");
        } else if !request.email.trim().validate_email() {
            error.push_violation("email", "field must be a valid email address");
        }

        if request.callback_uri.trim().is_empty() {
            error.push_violation("callback_uri", "field is required and cannot be empty");
        }

        if error.has_violations() {
            return Err(error);
        }

        Ok(())
    }

    /// Cleared so the address can be invited again, rather than refused for
    /// a share nobody can still accept.
    async fn clear_expired_invitation(&self, schema_id: &str, email: &str) -> Result<(), Error> {
        let expired = self
            .shares
            .invitations(schema_id)
            .await
            .map_err(to_error)?
            .into_iter()
            .find(|invitation| invitation.email == email && invitation.is_expired_at(Utc::now()));

        if let Some(invitation) = expired {
            self.shares
                .delete_invitation(schema_id, invitation.id)
                .await
                .map_err(to_error)?;
        }

        Ok(())
    }

    async fn invite(
        &self,
        caller: Uuid,
        grant: Grant,
        schema_id: &str,
        request: ShareRequest,
        email: String,
    ) -> Result<Invitation, Error> {
        self.clear_expired_invitation(schema_id, &email).await?;

        let token = crypto::random_token();

        // Only the digest is stored, the raw token travels by email.
        let invitation = self
            .shares
            .create_invitation(InvitationDraft {
                schema_id: schema_id.to_owned(),
                email: email.clone(),
                role: request.role,
                token_hash: crypto::hash_token(&token),
                invited_by: caller,
                expires_at: Utc::now() + Duration::days(INVITATION_TTL_DAYS),
            })
            .await
            .map_err(to_error)?;

        let schema = self
            .schemas
            .find_by_id(grant.owner, schema_id)
            .await
            .map_err(to_error)?
            .ok_or_else(|| Error::not_found("schema does not exist"))?;

        let (inviter, picture) = self.sender(caller).await?;

        let data = HashMap::from([
            ("schemaName".to_owned(), schema.name.clone()),
            ("inviter".to_owned(), inviter.clone()),
            ("initials".to_owned(), initials(&inviter)),
            ("avatar".to_owned(), avatar(picture.as_deref(), &inviter)),
            ("role".to_owned(), request.role.to_string()),
            (
                "callbackURI".to_owned(),
                format!("{}{token}", request.callback_uri),
            ),
        ]);

        // A mail that never left is an invitation nobody can accept, so it is
        // taken back and the owner can try again.
        if let Err(failure) = self
            .mailer
            .send_html(
                &email,
                &format!("{inviter} shared \"{}\" with you", schema.name),
                INVITATION_TEMPLATE,
                data,
            )
            .await
        {
            self.shares
                .delete_invitation(schema_id, invitation.id)
                .await
                .map_err(to_error)?;

            return Err(Error::unknown(failure));
        }

        tracing::info!(schema.id = %schema_id, "invitation sent");

        Ok(invitation)
    }
}
