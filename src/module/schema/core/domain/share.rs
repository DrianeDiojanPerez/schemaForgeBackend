use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::schema::SchemaSummary;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShareRole {
    Viewer,
    Editor,
}

impl ShareRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            ShareRole::Viewer => "viewer",
            ShareRole::Editor => "editor",
        }
    }

    pub const VARIANTS: [&'static str; 2] = ["viewer", "editor"];

    pub const fn can_edit(self) -> bool {
        matches!(self, ShareRole::Editor)
    }
}

impl fmt::Display for ShareRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ShareRole {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "viewer" => Ok(ShareRole::Viewer),
            "editor" => Ok(ShareRole::Editor),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Owner,
    Shared(ShareRole),
}

impl Access {
    pub const fn as_str(self) -> &'static str {
        match self {
            Access::Owner => "owner",
            Access::Shared(role) => role.as_str(),
        }
    }

    pub const fn can_edit(self) -> bool {
        match self {
            Access::Owner => true,
            Access::Shared(role) => role.can_edit(),
        }
    }

    pub const fn is_owner(self) -> bool {
        matches!(self, Access::Owner)
    }
}

impl fmt::Display for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    pub owner: Uuid,
    pub access: Access,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collaborator {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    /// The picture Google sent at sign-in, when there was one.
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    pub schema_id: String,
    pub user_id: Uuid,
    pub role: ShareRole,
    pub shared_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    pub id: Uuid,
    pub schema_id: String,
    pub email: String,
    pub role: ShareRole,
    pub invited_by: Uuid,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl Invitation {
    pub fn is_expired_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SharedSchema {
    pub summary: SchemaSummary,
    pub role: ShareRole,
    pub owner_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_role_in_any_case() {
        assert_eq!("viewer".parse(), Ok(ShareRole::Viewer));
        assert_eq!(" Editor ".parse(), Ok(ShareRole::Editor));
        assert!("owner".parse::<ShareRole>().is_err());
    }

    #[test]
    fn renders_back_to_the_name_it_parsed_from() {
        for name in ShareRole::VARIANTS {
            let role: ShareRole = name.parse().expect("the variant list should parse");

            assert_eq!(role.to_string(), name);
        }
    }

    #[test]
    fn only_the_owner_and_an_editor_may_write() {
        assert!(Access::Owner.can_edit());
        assert!(Access::Shared(ShareRole::Editor).can_edit());
        assert!(!Access::Shared(ShareRole::Viewer).can_edit());
    }

    #[test]
    fn an_editor_is_still_not_the_owner() {
        assert!(Access::Owner.is_owner());
        assert!(!Access::Shared(ShareRole::Editor).is_owner());
        assert_eq!(Access::Shared(ShareRole::Editor).as_str(), "editor");
    }

    #[test]
    fn an_invitation_expires_on_the_instant_not_after_it() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).expect("a fixed instant");
        let invitation = Invitation {
            id: Uuid::nil(),
            schema_id: "s1".to_owned(),
            email: "cat@example.com".to_owned(),
            role: ShareRole::Viewer,
            invited_by: Uuid::nil(),
            expires_at: now,
            created_at: now,
        };

        assert!(invitation.is_expired_at(now));
        assert!(!invitation.is_expired_at(now - chrono::Duration::seconds(1)));
    }
}
