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

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use chrono::DateTime;

    use crate::module::schema::core::domain::{Access, Entity, Schema, SchemaDraft, SchemaSummary};
    use crate::module::schema::core::service::fake_shares::{FakeDirectory, FakeShares};
    use crate::package::emailer::EmailerError;
    use crate::package::errdef::code;

    const CALLBACK: &str = "https://app.test/invitations/";

    fn owner() -> Uuid {
        Uuid::from_u128(1)
    }

    fn bob() -> Uuid {
        Uuid::from_u128(2)
    }

    fn stranger() -> Uuid {
        Uuid::from_u128(3)
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("a fixed instant")
    }

    fn person(id: Uuid, email: &str, name: &str) -> Collaborator {
        Collaborator {
            id,
            email: email.to_owned(),
            name: name.to_owned(),
            avatar_url: None,
        }
    }

    #[derive(Default)]
    struct FakeSchemas {
        schemas: Mutex<Vec<(Uuid, Schema)>>,
    }

    impl FakeSchemas {
        fn holding(owner: Uuid, schemas: Vec<Schema>) -> Self {
            Self {
                schemas: Mutex::new(schemas.into_iter().map(|schema| (owner, schema)).collect()),
            }
        }
    }

    #[async_trait]
    impl SchemaRepository for FakeSchemas {
        async fn index(
            &self,
            owner: Uuid,
            _request: &ListRequest,
        ) -> Result<(Vec<SchemaSummary>, i64), DomainError> {
            let mine: Vec<SchemaSummary> = self
                .schemas
                .lock()
                .unwrap()
                .iter()
                .filter(|(by, _)| *by == owner)
                .map(|(_, schema)| schema.summary())
                .collect();
            let total = mine.len() as i64;

            Ok((mine, total))
        }

        async fn create(&self, _owner: Uuid, _draft: SchemaDraft) -> Result<Schema, DomainError> {
            unimplemented!("sharing never creates a schema")
        }

        async fn find_by_id(&self, owner: Uuid, id: &str) -> Result<Option<Schema>, DomainError> {
            Ok(self
                .schemas
                .lock()
                .unwrap()
                .iter()
                .find(|(by, schema)| *by == owner && schema.id == id)
                .map(|(_, schema)| schema.clone()))
        }

        async fn find_by_name(
            &self,
            _owner: Uuid,
            _name: &str,
        ) -> Result<Option<Schema>, DomainError> {
            unimplemented!("sharing never looks a schema up by name")
        }

        async fn replace(
            &self,
            _owner: Uuid,
            _id: &str,
            _draft: SchemaDraft,
        ) -> Result<Schema, DomainError> {
            unimplemented!("sharing never replaces a schema")
        }

        async fn delete(&self, _owner: Uuid, _id: &str) -> Result<(), DomainError> {
            unimplemented!("sharing never deletes a schema")
        }
    }

    type Sent = (String, String, HashMap<String, String>);

    #[derive(Default)]
    struct FakeMailer {
        sent: Mutex<Vec<Sent>>,
        down: bool,
    }

    impl FakeMailer {
        fn down() -> Self {
            Self {
                sent: Mutex::default(),
                down: true,
            }
        }

        fn last(&self) -> Sent {
            self.sent
                .lock()
                .unwrap()
                .last()
                .cloned()
                .expect("a mail should have been sent")
        }
    }

    #[async_trait]
    impl Emailer for FakeMailer {
        async fn send_html(
            &self,
            to: &str,
            subject: &str,
            _template_name: &str,
            data: HashMap<String, String>,
        ) -> Result<(), EmailerError> {
            if self.down {
                return Err(EmailerError::TemplateNotFound("smtp is down".to_owned()));
            }

            self.sent
                .lock()
                .unwrap()
                .push((to.to_owned(), subject.to_owned(), data));

            Ok(())
        }
    }

    fn stored(id: &str, name: &str) -> Schema {
        Schema::new(
            id,
            SchemaDraft {
                name: name.to_owned(),
                description: String::new(),
                entities: vec![Entity::new("e1", "users")],
                relationships: Vec::new(),
            },
            now(),
        )
    }

    struct World {
        schemas: Arc<FakeSchemas>,
        shares: Arc<FakeShares>,
        mailer: Arc<FakeMailer>,
        service: SharingServiceImpl,
    }

    /// The owner holds "blog" and "shop"; bob has an account and the
    /// stranger has one too, while cat@example.com has none.
    async fn world_with(mailer: FakeMailer) -> World {
        let schemas = Arc::new(FakeSchemas::holding(
            owner(),
            vec![stored("s1", "blog"), stored("s2", "shop")],
        ));
        let shares = Arc::new(FakeShares::default());
        shares.mirror(owner(), schemas.as_ref()).await;
        let mailer = Arc::new(mailer);

        let service = SharingServiceImpl::new(
            schemas.clone(),
            shares.clone(),
            Arc::new(FakeDirectory::knowing(vec![
                person(owner(), "ada@example.com", "Ada Lovelace"),
                person(bob(), "bob@example.com", "Bob Builder"),
                person(stranger(), "sam@example.com", "Sam Stranger"),
            ])),
            mailer.clone(),
        );

        World {
            schemas,
            shares,
            mailer,
            service,
        }
    }

    async fn world() -> World {
        world_with(FakeMailer::default()).await
    }

    fn share_with(email: &str, role: ShareRole) -> ShareRequest {
        ShareRequest {
            email: email.to_owned(),
            role,
            callback_uri: CALLBACK.to_owned(),
        }
    }

    #[test]
    fn the_badge_is_the_picture_when_there_is_one_and_the_initials_otherwise() {
        assert_eq!(avatar(None, "Ada Lovelace"), "AL");
        assert_eq!(avatar(Some("  "), "Ada Lovelace"), "AL");
        assert_eq!(
            avatar(Some("http://plain.example/a.png"), "Ada Lovelace"),
            "AL"
        );
        let tag = avatar(Some("https://lh3.example/a=s96\"<"), "Ada Lovelace");
        assert!(tag.starts_with("<img src=\"https://lh3.example/a=s96&quot;&lt;\""));
        assert!(!tag.contains("AL"));
    }

    #[test]
    fn initials_come_from_the_name_or_failing_that_the_address() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("ada"), "A");
        assert_eq!(initials("  bob  builder  jr "), "BB");
        assert_eq!(initials("cat@example.com"), "CE");
    }

    #[tokio::test]
    async fn sharing_with_someone_who_has_an_account_grants_access_now() {
        let world = world().await;

        let shared = world
            .service
            .share(
                owner(),
                "s1",
                share_with(" Bob@Example.com ", ShareRole::Editor),
            )
            .await
            .expect("the share should succeed");

        let Shared::Granted(member) = shared else {
            panic!("an address with an account is granted, not invited");
        };

        assert_eq!(member.user.name, "Bob Builder");
        assert_eq!(member.share.role, ShareRole::Editor);
        assert_eq!(
            world
                .shares
                .grant(bob(), "s1")
                .await
                .expect("the lookup should succeed")
                .map(|grant| grant.access),
            Some(Access::Shared(ShareRole::Editor))
        );

        let (to, subject, data) = world.mailer.last();

        assert_eq!(to, "bob@example.com");
        assert_eq!(subject, "Ada Lovelace shared \"blog\" with you");
        assert_eq!(data["inviter"], "Ada Lovelace");
        assert_eq!(data["verb"], "edit");
        assert_eq!(data["appURI"], "https://app.test/");
        assert!(!data.contains_key("callbackURI"), "a member needs no token");
    }

    #[tokio::test]
    async fn a_shared_mail_that_did_not_leave_does_not_undo_the_share() {
        let world = world_with(FakeMailer::down()).await;

        let shared = world
            .service
            .share(
                owner(),
                "s1",
                share_with("bob@example.com", ShareRole::Viewer),
            )
            .await
            .expect("the share should stand even when the mail fails");

        assert!(matches!(shared, Shared::Granted(_)));
    }

    #[test]
    fn the_app_is_the_origin_of_the_invitation_link() {
        assert_eq!(
            app_uri("https://app.test/invitations/"),
            "https://app.test/"
        );
        assert_eq!(
            app_uri("http://localhost:3100/invitations/"),
            "http://localhost:3100/"
        );
        assert_eq!(app_uri("https://app.test"), "https://app.test/");
    }

    #[tokio::test]
    async fn sharing_with_an_address_nobody_has_sends_an_invitation() {
        let world = world().await;

        let shared = world
            .service
            .share(
                owner(),
                "s1",
                share_with("cat@example.com", ShareRole::Viewer),
            )
            .await
            .expect("the share should succeed");

        let Shared::Invited(invitation) = shared else {
            panic!("an address without an account is invited");
        };

        assert_eq!(invitation.email, "cat@example.com");
        assert_eq!(invitation.role, ShareRole::Viewer);

        let (to, subject, data) = world.mailer.last();

        assert_eq!(to, "cat@example.com");
        assert_eq!(subject, "Ada Lovelace shared \"blog\" with you");
        assert_eq!(data["schemaName"], "blog");
        assert_eq!(data["inviter"], "Ada Lovelace");
        assert_eq!(data["initials"], "AL");
        assert_eq!(data["role"], "viewer");
        assert!(
            data["callbackURI"].starts_with(CALLBACK),
            "the link goes where the caller said: {}",
            data["callbackURI"]
        );
        assert_ne!(
            Some(data["callbackURI"][CALLBACK.len()..].to_owned()),
            world.shares.token_of(invitation.id),
            "the token in the mail is not the digest that was stored"
        );
    }

    #[tokio::test]
    async fn an_invitation_nobody_received_is_taken_back() {
        let world = world_with(FakeMailer::down()).await;

        let error = world
            .service
            .share(
                owner(),
                "s1",
                share_with("cat@example.com", ShareRole::Viewer),
            )
            .await
            .expect_err("the share should fail");

        assert_eq!(error.app_code(), code::UNKNOWN);
        assert!(
            world.shares.invitations_for("cat@example.com").is_empty(),
            "the owner can try again once mail is back"
        );
    }

    #[tokio::test]
    async fn only_the_owner_shares() {
        let world = world().await;
        world
            .shares
            .create_share("s1", bob(), ShareRole::Editor, owner())
            .await
            .expect("the share should be written");

        assert_eq!(
            world
                .service
                .share(
                    bob(),
                    "s1",
                    share_with("sam@example.com", ShareRole::Viewer)
                )
                .await
                .expect_err("an editor may not share")
                .app_code(),
            code::FORBIDDEN
        );
        assert_eq!(
            world
                .service
                .share(
                    stranger(),
                    "s1",
                    share_with("bob@example.com", ShareRole::Viewer)
                )
                .await
                .expect_err("a stranger cannot tell the schema exists")
                .app_code(),
            code::NOT_FOUND
        );
        assert_eq!(
            world
                .service
                .people(stranger(), "s1")
                .await
                .expect_err("a stranger cannot list who is in either")
                .app_code(),
            code::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn a_member_sees_who_else_is_in_but_not_who_is_still_invited() {
        let world = world().await;

        world
            .service
            .share(
                owner(),
                "s1",
                share_with("bob@example.com", ShareRole::Editor),
            )
            .await
            .expect("the share should succeed");
        world
            .service
            .share(
                owner(),
                "s1",
                share_with("cat@example.com", ShareRole::Viewer),
            )
            .await
            .expect("the invitation should succeed");

        let people = world
            .service
            .people(bob(), "s1")
            .await
            .expect("a member may list who is in");

        assert_eq!(people.members.len(), 1);
        assert_eq!(people.members[0].user.name, "Bob Builder");
        assert!(
            people.invitations.is_empty(),
            "invitations are the owner's to see"
        );
    }

    #[tokio::test]
    async fn a_schema_cannot_be_shared_with_its_owner_or_with_nothing() {
        let world = world().await;

        let with_self = world
            .service
            .share(
                owner(),
                "s1",
                share_with("ada@example.com", ShareRole::Viewer),
            )
            .await
            .expect_err("the owner already has it");
        let with_nothing = world
            .service
            .share(
                owner(),
                "s1",
                share_with("not an address", ShareRole::Viewer),
            )
            .await
            .expect_err("the address is not one");

        assert_eq!(with_self.app_code(), code::VALIDATION_FAILED);
        assert_eq!(with_nothing.app_code(), code::VALIDATION_FAILED);
    }

    #[tokio::test]
    async fn sharing_twice_is_a_conflict_that_names_the_address() {
        let world = world().await;

        world
            .service
            .share(
                owner(),
                "s1",
                share_with("bob@example.com", ShareRole::Viewer),
            )
            .await
            .expect("the first share should succeed");
        let again = world
            .service
            .share(
                owner(),
                "s1",
                share_with("bob@example.com", ShareRole::Editor),
            )
            .await
            .expect_err("the second share should fail");

        assert_eq!(again.app_code(), code::RESOURCE_CONFLICT);
        assert!(again.to_string().contains("bob@example.com"));

        world
            .service
            .share(
                owner(),
                "s1",
                share_with("cat@example.com", ShareRole::Viewer),
            )
            .await
            .expect("the first invitation should succeed");
        let again = world
            .service
            .share(
                owner(),
                "s1",
                share_with("cat@example.com", ShareRole::Viewer),
            )
            .await
            .expect_err("the second invitation should fail");

        assert_eq!(again.app_code(), code::RESOURCE_CONFLICT);
    }

    #[tokio::test]
    async fn the_people_list_holds_members_and_the_invited() {
        let world = world().await;

        world
            .service
            .share(
                owner(),
                "s1",
                share_with("bob@example.com", ShareRole::Editor),
            )
            .await
            .expect("the share should succeed");
        world
            .service
            .share(
                owner(),
                "s1",
                share_with("cat@example.com", ShareRole::Viewer),
            )
            .await
            .expect("the invitation should succeed");

        let people = world
            .service
            .people(owner(), "s1")
            .await
            .expect("the owner may list");

        assert_eq!(people.members.len(), 1);
        assert_eq!(people.members[0].user.email, "bob@example.com");
        assert_eq!(people.invitations.len(), 1);
        assert_eq!(people.invitations[0].email, "cat@example.com");
    }

    #[tokio::test]
    async fn a_role_can_be_changed_and_access_taken_away() {
        let world = world().await;
        world
            .service
            .share(
                owner(),
                "s1",
                share_with("bob@example.com", ShareRole::Viewer),
            )
            .await
            .expect("the share should succeed");

        let member = world
            .service
            .change_role(owner(), "s1", bob(), ShareRole::Editor)
            .await
            .expect("the owner may change a role");

        assert_eq!(member.share.role, ShareRole::Editor);
        assert_eq!(member.user.name, "Bob Builder");

        world
            .service
            .revoke(owner(), "s1", bob())
            .await
            .expect("the owner may revoke");

        assert!(world.shares.share_of("s1", bob()).is_none());
        assert_eq!(
            world
                .service
                .revoke(owner(), "s1", bob())
                .await
                .expect_err("there is nothing left to revoke")
                .app_code(),
            code::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn a_person_may_leave_but_not_push_someone_else_out() {
        let world = world().await;
        for who in [bob(), stranger()] {
            world
                .shares
                .create_share("s1", who, ShareRole::Editor, owner())
                .await
                .expect("the share should be written");
        }

        assert_eq!(
            world
                .service
                .revoke(bob(), "s1", stranger())
                .await
                .expect_err("an editor may not revoke another")
                .app_code(),
            code::FORBIDDEN
        );

        world
            .service
            .revoke(bob(), "s1", bob())
            .await
            .expect("anyone may give up their own access");

        assert!(world.shares.share_of("s1", bob()).is_none());
        assert!(world.shares.share_of("s1", stranger()).is_some());
    }

    #[tokio::test]
    async fn an_invitation_can_be_cancelled_by_the_owner_alone() {
        let world = world().await;
        let Shared::Invited(invitation) = world
            .service
            .share(
                owner(),
                "s1",
                share_with("cat@example.com", ShareRole::Viewer),
            )
            .await
            .expect("the invitation should succeed")
        else {
            panic!("an unknown address is invited");
        };

        assert_eq!(
            world
                .service
                .cancel_invitation(bob(), "s1", invitation.id)
                .await
                .expect_err("a stranger cannot tell the schema exists")
                .app_code(),
            code::NOT_FOUND
        );

        world
            .service
            .cancel_invitation(owner(), "s1", invitation.id)
            .await
            .expect("the owner may cancel");

        assert!(world.shares.invitations_for("cat@example.com").is_empty());
    }

    async fn invited_cat(world: &World) -> (Uuid, String) {
        let (_, _, data) = {
            world
                .service
                .share(
                    owner(),
                    "s1",
                    share_with("cat@example.com", ShareRole::Editor),
                )
                .await
                .expect("the invitation should succeed");
            world.mailer.last()
        };
        let token = data["callbackURI"][CALLBACK.len()..].to_owned();

        (Uuid::from_u128(4), token)
    }

    fn knowing_cat(world: World, cat: Uuid) -> SharingServiceImpl {
        SharingServiceImpl::new(
            world.schemas.clone(),
            world.shares.clone(),
            Arc::new(FakeDirectory::knowing(vec![
                person(owner(), "ada@example.com", "Ada Lovelace"),
                person(bob(), "bob@example.com", "Bob Builder"),
                person(cat, "Cat@Example.com", "Cat"),
            ])),
            world.mailer.clone(),
        )
    }

    #[tokio::test]
    async fn opening_the_link_as_the_invited_address_grants_the_role_once() {
        let world = world().await;
        let (cat, token) = invited_cat(&world).await;
        let shares = world.shares.clone();
        let service = knowing_cat(world, cat);

        let listing = service
            .accept_invitation(cat, &token)
            .await
            .expect("the invited person may accept");

        assert_eq!(listing.summary.name, "blog");
        assert_eq!(listing.role, ShareRole::Editor);
        assert_eq!(listing.owner.name, "Ada Lovelace");
        assert_eq!(
            shares.share_of("s1", cat).map(|share| share.role),
            Some(ShareRole::Editor)
        );
        assert_eq!(
            service
                .accept_invitation(cat, &token)
                .await
                .expect_err("a link works once")
                .app_code(),
            code::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn a_forwarded_link_does_not_work_for_someone_else() {
        let world = world().await;
        let (cat, token) = invited_cat(&world).await;
        let shares = world.shares.clone();
        let service = knowing_cat(world, cat);

        assert_eq!(
            service
                .accept_invitation(bob(), &token)
                .await
                .expect_err("bob was not invited")
                .app_code(),
            code::FORBIDDEN
        );
        assert!(
            shares.share_of("s1", bob()).is_none(),
            "nothing was granted"
        );
        assert_eq!(
            service
                .accept_invitation(cat, "not-a-token")
                .await
                .expect_err("an unknown token names nothing")
                .app_code(),
            code::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn an_invitation_that_ran_out_is_refused_and_cleared() {
        let world = world().await;
        world
            .shares
            .create_invitation(InvitationDraft {
                schema_id: "s1".to_owned(),
                email: "bob@example.com".to_owned(),
                role: ShareRole::Viewer,
                token_hash: crypto::hash_token("old"),
                invited_by: owner(),
                expires_at: Utc::now() - Duration::days(1),
            })
            .await
            .expect("the invitation should be written");

        let error = world
            .service
            .accept_invitation(bob(), "old")
            .await
            .expect_err("the invitation ran out");

        assert_eq!(error.app_code(), code::BAD_REQUEST);
        assert!(world.shares.invitations_for("bob@example.com").is_empty());

        // The owner can invite the address again rather than being told it
        // is already invited.
        let Shared::Granted(_) = world
            .service
            .share(
                owner(),
                "s1",
                share_with("bob@example.com", ShareRole::Viewer),
            )
            .await
            .expect("sharing again should succeed")
        else {
            panic!("bob has an account by now");
        };
    }

    #[tokio::test]
    async fn shared_with_me_lists_what_others_let_me_in_on() {
        let world = world().await;
        for schema_id in ["s1", "s2"] {
            world
                .shares
                .create_share(schema_id, bob(), ShareRole::Viewer, owner())
                .await
                .expect("the share should be written");
        }

        let page = world
            .service
            .shared_with_me(bob(), ListRequest::paged(1, 1))
            .await
            .expect("the listing should succeed");

        assert_eq!(page.meta.total_count, 2);
        assert_eq!(page.data.len(), 1, "the page is capped at the page size");
        assert_eq!(page.data[0].summary.name, "blog");
        assert_eq!(page.data[0].role, ShareRole::Viewer);
        assert_eq!(page.data[0].owner.email, "ada@example.com");

        let nothing = world
            .service
            .shared_with_me(stranger(), ListRequest::default())
            .await
            .expect("the listing should succeed");

        assert_eq!(nothing.meta.total_count, 0);
    }
}
