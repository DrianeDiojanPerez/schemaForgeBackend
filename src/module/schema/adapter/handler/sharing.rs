use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json as AxumJson;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::module::schema::adapter::handler::mapper::{
    self, Invitation, Member, SharedSchema, Violations,
};
use crate::module::schema::core::domain::ShareRole;
use crate::module::schema::core::ports::{ShareRequest, Shared, SharingService};
use crate::package::auth::AuthUser;
use crate::package::errdef::Error;
use crate::package::extract::Json;
use crate::package::pagination::ListRequest;
use crate::package::response::{self, Response};

pub type SharingState = Arc<dyn SharingService>;

#[derive(Debug, Deserialize)]
pub struct SharePayload {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub role: Option<String>,
    /// The token is appended to this.
    #[serde(default)]
    pub callback_uri: String,
}

#[derive(Debug, Deserialize)]
pub struct RolePayload {
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ShareOutcome {
    pub member: Option<Member>,
    pub invitation: Option<Invitation>,
}

#[derive(Debug, Serialize)]
pub struct People {
    pub members: Vec<Member>,
    pub invitations: Vec<Invitation>,
}

#[derive(Debug, Serialize)]
pub struct Revoked {
    pub message: &'static str,
    pub schema_id: String,
    pub user_id: String,
}

#[derive(Debug, Serialize)]
pub struct Cancelled {
    pub message: &'static str,
    pub schema_id: String,
    pub invitation_id: String,
}

/// A segment that is not an id names nothing, so it is a not found rather
/// than a bad request.
fn id_from(value: &str, what: &str) -> Result<Uuid, Error> {
    Uuid::parse_str(value).map_err(|_| Error::not_found(format!("{what} does not exist")))
}

#[tracing::instrument(name = "SharingHandler.SharedWithMe", skip_all)]
pub async fn shared_with_me(
    State(service): State<SharingState>,
    caller: AuthUser,
    request: ListRequest,
) -> Result<AxumJson<Response<Vec<SharedSchema>>>, Error> {
    let page = service
        .shared_with_me(caller.id(), request)
        .await?
        .map(SharedSchema::from);

    Ok(response::ok_paginated(page.data, page.meta))
}

#[tracing::instrument(name = "SharingHandler.People", skip_all)]
pub async fn people(
    State(service): State<SharingState>,
    caller: AuthUser,
    Path(schema_id): Path<String>,
) -> Result<AxumJson<Response<People>>, Error> {
    let people = service.people(caller.id(), &schema_id).await?;

    Ok(response::ok(People {
        members: people.members.into_iter().map(Member::from).collect(),
        invitations: people
            .invitations
            .into_iter()
            .map(Invitation::from)
            .collect(),
    }))
}

#[tracing::instrument(name = "SharingHandler.Share", skip_all)]
pub async fn share(
    State(service): State<SharingState>,
    caller: AuthUser,
    Path(schema_id): Path<String>,
    Json(payload): Json<SharePayload>,
) -> Result<AxumJson<Response<ShareOutcome>>, Error> {
    let mut violations = Violations::default();
    let role = mapper::share_role_from(payload.role.as_deref(), "role", &mut violations);

    violations.into_result()?;

    let shared = service
        .share(
            caller.id(),
            &schema_id,
            ShareRequest {
                email: payload.email,
                role: role.unwrap_or(ShareRole::Viewer),
                callback_uri: payload.callback_uri,
            },
        )
        .await?;

    Ok(response::ok(match shared {
        Shared::Granted(member) => ShareOutcome {
            member: Some(member.into()),
            invitation: None,
        },
        Shared::Invited(invitation) => ShareOutcome {
            member: None,
            invitation: Some(invitation.into()),
        },
    }))
}

#[tracing::instrument(name = "SharingHandler.ChangeRole", skip_all)]
pub async fn change_role(
    State(service): State<SharingState>,
    caller: AuthUser,
    Path((schema_id, user_id)): Path<(String, String)>,
    Json(payload): Json<RolePayload>,
) -> Result<AxumJson<Response<Member>>, Error> {
    let mut violations = Violations::default();
    let role = mapper::share_role_from(payload.role.as_deref(), "role", &mut violations);

    violations.into_result()?;

    let user_id = id_from(&user_id, "share")?;
    let member = service
        .change_role(
            caller.id(),
            &schema_id,
            user_id,
            role.unwrap_or(ShareRole::Viewer),
        )
        .await?;

    Ok(response::ok(Member::from(member)))
}

#[tracing::instrument(name = "SharingHandler.Revoke", skip_all)]
pub async fn revoke(
    State(service): State<SharingState>,
    caller: AuthUser,
    Path((schema_id, user_id)): Path<(String, String)>,
) -> Result<AxumJson<Response<Revoked>>, Error> {
    let user = id_from(&user_id, "share")?;

    service.revoke(caller.id(), &schema_id, user).await?;

    Ok(response::ok(Revoked {
        message: "Access revoked successfully",
        schema_id,
        user_id,
    }))
}

#[tracing::instrument(name = "SharingHandler.CancelInvitation", skip_all)]
pub async fn cancel_invitation(
    State(service): State<SharingState>,
    caller: AuthUser,
    Path((schema_id, invitation_id)): Path<(String, String)>,
) -> Result<AxumJson<Response<Cancelled>>, Error> {
    let invitation = id_from(&invitation_id, "invitation")?;

    service
        .cancel_invitation(caller.id(), &schema_id, invitation)
        .await?;

    Ok(response::ok(Cancelled {
        message: "Invitation cancelled successfully",
        schema_id,
        invitation_id,
    }))
}

#[tracing::instrument(name = "SharingHandler.AcceptInvitation", skip_all)]
pub async fn accept_invitation(
    State(service): State<SharingState>,
    caller: AuthUser,
    Path(token): Path<String>,
) -> Result<AxumJson<Response<SharedSchema>>, Error> {
    let listing = service.accept_invitation(caller.id(), &token).await?;

    Ok(response::ok(SharedSchema::from(listing)))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::Utc;

    use crate::module::schema::core::domain::{
        Collaborator, Invitation as DomainInvitation, Schema, SchemaDraft, Share,
    };
    use crate::module::schema::core::ports::{
        Member as DomainMember, People as DomainPeople, SharedListing,
    };
    use crate::package::auth::Identity;
    use crate::package::errdef::code;
    use crate::package::pagination::Data;

    #[derive(Default)]
    struct SpyService {
        fail_with: Option<fn() -> Error>,
        calls: Mutex<Vec<(Uuid, String)>>,
    }

    impl SpyService {
        fn failing(failure: fn() -> Error) -> Self {
            Self {
                fail_with: Some(failure),
                calls: Mutex::default(),
            }
        }

        fn guard(&self, caller: Uuid, what: &str) -> Result<(), Error> {
            self.calls
                .lock()
                .expect("the lock should be held")
                .push((caller, what.to_owned()));

            match self.fail_with {
                Some(failure) => Err(failure()),
                None => Ok(()),
            }
        }

        fn bob() -> Collaborator {
            Collaborator {
                id: Uuid::from_u128(2),
                email: "bob@example.com".to_owned(),
                name: "Bob Builder".to_owned(),
                avatar_url: None,
            }
        }

        fn member(role: ShareRole) -> DomainMember {
            DomainMember {
                share: Share {
                    schema_id: "s1".to_owned(),
                    user_id: Uuid::from_u128(2),
                    role,
                    shared_by: Uuid::from_u128(1),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                user: Self::bob(),
            }
        }

        fn invitation() -> DomainInvitation {
            DomainInvitation {
                id: Uuid::from_u128(9),
                schema_id: "s1".to_owned(),
                email: "cat@example.com".to_owned(),
                role: ShareRole::Viewer,
                invited_by: Uuid::from_u128(1),
                expires_at: Utc::now(),
                created_at: Utc::now(),
            }
        }

        fn listing() -> SharedListing {
            SharedListing {
                summary: Schema::new(
                    "s1",
                    SchemaDraft {
                        name: "blog".to_owned(),
                        ..SchemaDraft::default()
                    },
                    Utc::now(),
                )
                .summary(),
                role: ShareRole::Editor,
                owner: Self::bob(),
            }
        }
    }

    #[async_trait]
    impl SharingService for SpyService {
        async fn shared_with_me(
            &self,
            caller: Uuid,
            request: ListRequest,
        ) -> Result<Data<SharedListing>, Error> {
            self.guard(caller, "shared_with_me")?;

            Ok(Data::new(
                vec![Self::listing()],
                1,
                request.page,
                request.per_page,
            ))
        }

        async fn people(&self, caller: Uuid, _schema_id: &str) -> Result<DomainPeople, Error> {
            self.guard(caller, "people")?;

            Ok(DomainPeople {
                members: vec![Self::member(ShareRole::Viewer)],
                invitations: vec![Self::invitation()],
            })
        }

        async fn share(
            &self,
            caller: Uuid,
            _schema_id: &str,
            request: ShareRequest,
        ) -> Result<Shared, Error> {
            self.guard(caller, &format!("share:{}:{}", request.email, request.role))?;

            if request.email == "cat@example.com" {
                return Ok(Shared::Invited(Self::invitation()));
            }

            Ok(Shared::Granted(Self::member(request.role)))
        }

        async fn change_role(
            &self,
            caller: Uuid,
            _schema_id: &str,
            _user_id: Uuid,
            role: ShareRole,
        ) -> Result<DomainMember, Error> {
            self.guard(caller, &format!("change_role:{role}"))?;

            Ok(Self::member(role))
        }

        async fn revoke(
            &self,
            caller: Uuid,
            _schema_id: &str,
            _user_id: Uuid,
        ) -> Result<(), Error> {
            self.guard(caller, "revoke")
        }

        async fn cancel_invitation(
            &self,
            caller: Uuid,
            _schema_id: &str,
            _invitation_id: Uuid,
        ) -> Result<(), Error> {
            self.guard(caller, "cancel_invitation")
        }

        async fn accept_invitation(
            &self,
            caller: Uuid,
            token: &str,
        ) -> Result<SharedListing, Error> {
            self.guard(caller, &format!("accept:{token}"))?;

            Ok(Self::listing())
        }
    }

    fn state(service: SpyService) -> State<SharingState> {
        State(Arc::new(service))
    }

    fn caller() -> AuthUser {
        AuthUser(Identity {
            id: Uuid::from_u128(1),
            email: "ada@example.com".to_owned(),
            user_name: "ada".to_owned(),
            first_name: "Ada".to_owned(),
            last_name: "Lovelace".to_owned(),
            avatar_url: None,
            password: String::new(),
            roles: Vec::new(),
        })
    }

    fn share_payload(email: &str, role: Option<&str>) -> SharePayload {
        SharePayload {
            email: email.to_owned(),
            role: role.map(str::to_owned),
            callback_uri: "https://app.test/invitations/".to_owned(),
        }
    }

    #[tokio::test]
    async fn sharing_with_an_account_answers_a_member_and_with_a_stranger_an_invitation() {
        let granted = share(
            state(SpyService::default()),
            caller(),
            Path("s1".to_owned()),
            Json(share_payload("bob@example.com", Some("editor"))),
        )
        .await
        .expect("the call should succeed")
        .0
        .data
        .expect("a body should come back");

        assert_eq!(
            granted.member.map(|member| member.role),
            Some("editor".to_owned())
        );
        assert!(granted.invitation.is_none());

        let invited = share(
            state(SpyService::default()),
            caller(),
            Path("s1".to_owned()),
            Json(share_payload("cat@example.com", Some("viewer"))),
        )
        .await
        .expect("the call should succeed")
        .0
        .data
        .expect("a body should come back");

        assert!(invited.member.is_none());
        assert_eq!(
            invited.invitation.map(|invitation| invitation.email),
            Some("cat@example.com".to_owned())
        );
    }

    #[tokio::test]
    async fn a_role_the_model_does_not_have_is_refused_before_the_service() {
        let spy = SpyService::failing(|| Error::unknown("the service should never be reached"));

        let error = share(
            state(spy),
            caller(),
            Path("s1".to_owned()),
            Json(share_payload("bob@example.com", Some("owner"))),
        )
        .await
        .expect_err("the call should fail");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);

        let error = change_role(
            state(SpyService::failing(|| {
                Error::unknown("the service should never be reached")
            })),
            caller(),
            Path(("s1".to_owned(), Uuid::from_u128(2).to_string())),
            Json(RolePayload { role: None }),
        )
        .await
        .expect_err("the call should fail");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);
    }

    #[tokio::test]
    async fn a_user_id_that_is_not_one_names_no_share() {
        let error = revoke(
            state(SpyService::failing(|| {
                Error::unknown("the service should never be reached")
            })),
            caller(),
            Path(("s1".to_owned(), "bob".to_owned())),
        )
        .await
        .expect_err("the call should fail");

        assert_eq!(error.app_code(), code::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_people_list_carries_both_members_and_invitations() {
        let people = people(
            state(SpyService::default()),
            caller(),
            Path("s1".to_owned()),
        )
        .await
        .expect("the call should succeed")
        .0
        .data
        .expect("a body should come back");

        assert_eq!(people.members[0].user.email, "bob@example.com");
        assert_eq!(people.invitations[0].email, "cat@example.com");
    }

    #[tokio::test]
    async fn every_handler_hands_the_caller_to_the_service() {
        let spy = Arc::new(SpyService::default());
        let state = || State(spy.clone() as SharingState);
        let bob = Uuid::from_u128(2).to_string();

        let _ = shared_with_me(state(), caller(), ListRequest::default())
            .await
            .expect("the listing should succeed");
        let _ = people(state(), caller(), Path("s1".to_owned()))
            .await
            .expect("people should succeed");
        let _ = share(
            state(),
            caller(),
            Path("s1".to_owned()),
            Json(share_payload("bob@example.com", Some("viewer"))),
        )
        .await
        .expect("share should succeed");
        let _ = change_role(
            state(),
            caller(),
            Path(("s1".to_owned(), bob.clone())),
            Json(RolePayload {
                role: Some("editor".to_owned()),
            }),
        )
        .await
        .expect("change_role should succeed");
        let _ = revoke(state(), caller(), Path(("s1".to_owned(), bob)))
            .await
            .expect("revoke should succeed");
        let _ = cancel_invitation(
            state(),
            caller(),
            Path(("s1".to_owned(), Uuid::from_u128(9).to_string())),
        )
        .await
        .expect("cancel should succeed");
        let accepted = accept_invitation(state(), caller(), Path("tok".to_owned()))
            .await
            .expect("accept should succeed")
            .0
            .data
            .expect("a body should come back");

        assert_eq!(accepted.role, "editor");
        assert_eq!(accepted.owner.name, "Bob Builder");

        let calls = spy.calls.lock().expect("the lock should be held");

        assert_eq!(calls.len(), 7);
        assert!(calls.iter().all(|(seen, _)| *seen == caller().id()));
        assert!(calls.iter().any(|(_, what)| what == "accept:tok"));
        assert!(calls.iter().any(|(_, what)| what == "change_role:editor"));
    }
}
