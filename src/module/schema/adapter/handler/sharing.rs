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
