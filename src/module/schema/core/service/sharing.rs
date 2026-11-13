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

#[async_trait]
impl SharingService for SharingServiceImpl {
    #[tracing::instrument(name = "SharingService.SharedWithMe", skip_all)]
    async fn shared_with_me(
        &self,
        caller: Uuid,
        request: ListRequest,
    ) -> Result<Data<SharedListing>, Error> {
        let (shared, total) = self
            .shares
            .shared_with(caller, &request)
            .await
            .map_err(to_error)?;

        let owners = self
            .people_by_id(shared.iter().map(|entry| entry.owner_id).collect())
            .await?;

        let listings = shared
            .into_iter()
            .map(|entry| SharedListing {
                owner: owners
                    .get(&entry.owner_id)
                    .cloned()
                    .unwrap_or_else(|| unnamed(entry.owner_id)),
                summary: entry.summary,
                role: entry.role,
            })
            .collect();

        tracing::debug!(total, "shared schemas listed");

        Ok(Data::new(listings, total, request.page, request.per_page))
    }

    #[tracing::instrument(name = "SharingService.People", skip_all, fields(schema.id = %schema_id))]
    async fn people(&self, caller: Uuid, schema_id: &str) -> Result<People, Error> {
        let grant = self.grant(caller, schema_id).await?;

        let shares = self.shares.shares(schema_id).await.map_err(to_error)?;
        // Who else is in is anyone's to see, the way a shared file names its
        // people. Who has been asked and not yet answered is the owner's.
        let invitations = if grant.access.is_owner() {
            self.shares.invitations(schema_id).await.map_err(to_error)?
        } else {
            Vec::new()
        };

        let users = self
            .people_by_id(shares.iter().map(|share| share.user_id).collect())
            .await?;

        let members = shares
            .into_iter()
            .map(|share| Member {
                user: users
                    .get(&share.user_id)
                    .cloned()
                    .unwrap_or_else(|| unnamed(share.user_id)),
                share,
            })
            .collect();

        Ok(People {
            members,
            invitations,
        })
    }

    #[tracing::instrument(name = "SharingService.Share", skip_all, fields(schema.id = %schema_id))]
    async fn share(
        &self,
        caller: Uuid,
        schema_id: &str,
        request: ShareRequest,
    ) -> Result<Shared, Error> {
        Self::check_share(&request)?;

        let grant = self.owned(caller, schema_id).await?;
        let email = clean_email(&request.email);

        let Some(person) = self
            .directory
            .find_by_email(&email)
            .await
            .map_err(to_error)?
        else {
            return Ok(Shared::Invited(
                self.invite(caller, grant, schema_id, request, email)
                    .await?,
            ));
        };

        if person.id == caller {
            return Err(Error::validation("failed payload validation")
                .add_violation("email", "a schema cannot be shared with its owner"));
        }

        let share = self
            .shares
            .create_share(schema_id, person.id, request.role, caller)
            .await
            .map_err(|failure| match failure {
                DomainError::AlreadyShared(_) => to_error(DomainError::AlreadyShared(email)),
                other => to_error(other),
            })?;

        tracing::info!(user.id = %person.id, role = %share.role, "schema shared");

        // They already have the schema, so a mail that never left changes
        // nothing: it is logged and the share stands.
        if !person.email.is_empty() {
            let (sharer, picture) = self.sender(caller).await?;
            let schema = self
                .schemas
                .find_by_id(grant.owner, schema_id)
                .await
                .map_err(to_error)?
                .ok_or_else(|| Error::not_found("schema does not exist"))?;

            let data = HashMap::from([
                ("schemaName".to_owned(), schema.name.clone()),
                ("inviter".to_owned(), sharer.clone()),
                ("avatar".to_owned(), avatar(picture.as_deref(), &sharer)),
                (
                    "verb".to_owned(),
                    match share.role {
                        ShareRole::Editor => "edit".to_owned(),
                        ShareRole::Viewer => "view".to_owned(),
                    },
                ),
                ("appURI".to_owned(), app_uri(&request.callback_uri)),
            ]);

            if let Err(failure) = self
                .mailer
                .send_html(
                    &person.email,
                    &format!("{sharer} shared \"{}\" with you", schema.name),
                    SHARED_TEMPLATE,
                    data,
                )
                .await
            {
                tracing::warn!(error = %failure, "the shared mail did not leave");
            }
        }

        Ok(Shared::Granted(Member {
            share,
            user: person,
        }))
    }

    #[tracing::instrument(name = "SharingService.ChangeRole", skip_all, fields(schema.id = %schema_id))]
    async fn change_role(
        &self,
        caller: Uuid,
        schema_id: &str,
        user_id: Uuid,
        role: ShareRole,
    ) -> Result<Member, Error> {
        self.owned(caller, schema_id).await?;

        let share = self
            .shares
            .update_share(schema_id, user_id, role)
            .await
            .map_err(to_error)?;

        tracing::info!(%user_id, %role, "share role changed");

        self.member(share).await
    }

    #[tracing::instrument(name = "SharingService.Revoke", skip_all, fields(schema.id = %schema_id))]
    async fn revoke(&self, caller: Uuid, schema_id: &str, user_id: Uuid) -> Result<(), Error> {
        let grant = self.grant(caller, schema_id).await?;

        // Giving up one's own access takes no permission. Taking someone
        // else's does.
        if user_id != caller && !grant.access.is_owner() {
            return Err(Error::forbidden("only the owner may share this schema"));
        }

        self.shares
            .delete_share(schema_id, user_id)
            .await
            .map_err(to_error)?;

        tracing::info!(%user_id, "share revoked");

        Ok(())
    }

    #[tracing::instrument(name = "SharingService.CancelInvitation", skip_all, fields(schema.id = %schema_id))]
    async fn cancel_invitation(
        &self,
        caller: Uuid,
        schema_id: &str,
        invitation_id: Uuid,
    ) -> Result<(), Error> {
        self.owned(caller, schema_id).await?;

        self.shares
            .delete_invitation(schema_id, invitation_id)
            .await
            .map_err(to_error)?;

        tracing::info!(%invitation_id, "invitation cancelled");

        Ok(())
    }

    #[tracing::instrument(name = "SharingService.AcceptInvitation", skip_all)]
    async fn accept_invitation(&self, caller: Uuid, token: &str) -> Result<SharedListing, Error> {
        let invitation = self
            .shares
            .find_invitation_by_token(&crypto::hash_token(token))
            .await
            .map_err(to_error)?
            .ok_or_else(|| Error::not_found("invitation does not exist"))?;

        if invitation.is_expired_at(Utc::now()) {
            self.shares
                .delete_invitation(&invitation.schema_id, invitation.id)
                .await
                .map_err(to_error)?;

            return Err(Error::bad_request("invitation has expired"));
        }

        // The link is only good for the address it went to, so a forwarded
        // email does not hand the schema to whoever opens it.
        let person = self
            .people_by_id(vec![caller])
            .await?
            .remove(&caller)
            .ok_or_else(|| Error::unauthorized("the caller is not a known user"))?;

        if !person.email.eq_ignore_ascii_case(&invitation.email) {
            return Err(Error::forbidden(
                "this invitation was sent to another address",
            ));
        }

        let share = self
            .shares
            .accept_invitation(invitation.id, caller)
            .await
            .map_err(to_error)?;

        let grant = self.grant(caller, &share.schema_id).await?;
        let schema = self
            .schemas
            .find_by_id(grant.owner, &share.schema_id)
            .await
            .map_err(to_error)?
            .ok_or_else(|| Error::not_found("schema does not exist"))?;

        tracing::info!(schema.id = %share.schema_id, role = %share.role, "invitation accepted");

        self.listing_of(
            caller,
            SharedSchema {
                summary: schema.summary(),
                role: share.role,
                owner_id: grant.owner,
            },
        )
        .await
    }
}
