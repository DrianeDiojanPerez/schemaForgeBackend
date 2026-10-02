//! The Google half of sign in: the consent link the browser is sent to, and
//! the exchange that turns the code it comes back with into an account.

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;

use crate::package::errdef::Error;

const CONSENT_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "openid email profile";

const UNREACHABLE: &str = "google could not be reached";
const INVALID_CODE: &str = "google rejected the sign in code";

#[derive(Debug, Clone)]
pub struct GoogleAccount {
    pub email: String,
    pub email_verified: bool,
    pub name: String,
}

#[async_trait]
pub trait GoogleIdentity: Send + Sync {
    fn consent_url(&self, state: &str) -> String;
    async fn account_for(&self, code: &str) -> Result<GoogleAccount, Error>;
}

#[derive(Clone)]
pub struct GoogleCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

pub struct GoogleOAuth {
    credentials: GoogleCredentials,
    http: reqwest::Client,
    token_endpoint: String,
}

impl GoogleOAuth {
    pub fn new(credentials: GoogleCredentials) -> Self {
        Self {
            credentials,
            http: reqwest::Client::new(),
            token_endpoint: TOKEN_ENDPOINT.to_owned(),
        }
    }
}

#[async_trait]
impl GoogleIdentity for GoogleOAuth {
    fn consent_url(&self, state: &str) -> String {
        let query = serde_urlencoded::to_string([
            ("client_id", self.credentials.client_id.as_str()),
            ("redirect_uri", self.credentials.redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("access_type", "online"),
            ("prompt", "select_account"),
            ("state", state),
        ])
        .unwrap_or_default();

        format!("{CONSENT_ENDPOINT}?{query}")
    }

    async fn account_for(&self, code: &str) -> Result<GoogleAccount, Error> {
        let response = self
            .http
            .post(&self.token_endpoint)
            .form(&[
                ("code", code),
                ("client_id", self.credentials.client_id.as_str()),
                ("client_secret", self.credentials.client_secret.as_str()),
                ("redirect_uri", self.credentials.redirect_uri.as_str()),
                ("grant_type", "authorization_code"),
            ])
            .send()
            .await
            .map_err(|err| Error::unavailable(UNREACHABLE).with_cause(err))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| Error::unavailable(UNREACHABLE).with_cause(err))?;

        // Google answering at all rules out a transport problem, so only its
        // own failure is still worth a retry. A rejected code is not.
        if status.is_server_error() {
            return Err(Error::unavailable(UNREACHABLE).with_cause(body));
        }

        if !status.is_success() {
            return Err(Error::unauthorized(INVALID_CODE).with_cause(body));
        }

        let exchange: Exchange = serde_json::from_str(&body).map_err(Error::unknown)?;

        account_from(&exchange.id_token)
    }
}

#[derive(Deserialize)]
struct Exchange {
    id_token: String,
}

#[derive(Deserialize)]
struct IdToken {
    email: String,
    #[serde(default)]
    email_verified: bool,
    #[serde(default)]
    name: String,
}

/// The token came straight from Google over TLS in answer to a request
/// carrying the client secret, so the payload is read rather than verified.
/// A signature check here would only prove the channel over again.
fn account_from(id_token: &str) -> Result<GoogleAccount, Error> {
    let payload = id_token
        .split('.')
        .nth(1)
        .ok_or_else(|| Error::unknown("the google id token has no payload"))?;

    let decoded = URL_SAFE_NO_PAD.decode(payload).map_err(Error::unknown)?;
    let claims: IdToken = serde_json::from_slice(&decoded).map_err(Error::unknown)?;

    Ok(GoogleAccount {
        email: claims.email,
        email_verified: claims.email_verified,
        name: claims.name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::package::errdef::code;

    fn oauth() -> GoogleOAuth {
        GoogleOAuth::new(GoogleCredentials {
            client_id: "123.apps.googleusercontent.com".to_owned(),
            client_secret: "GOCSPX-secret".to_owned(),
            redirect_uri: "http://localhost:3100/auth/google/callback".to_owned(),
        })
    }

    fn id_token(claims: serde_json::Value) -> String {
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(claims.to_string())
        )
    }

    #[test]
    fn the_consent_url_carries_everything_google_asks_for() {
        let url = oauth().consent_url("opaque-state");

        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains("client_id=123.apps.googleusercontent.com"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=openid+email+profile"));
        assert!(url.contains("access_type=online"));
        assert!(url.contains("prompt=select_account"));
    }

    #[test]
    fn the_redirect_uri_and_the_state_are_escaped() {
        let url = oauth().consent_url("a state/with symbols");

        assert!(
            url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A3100%2Fauth%2Fgoogle%2Fcallback")
        );
        assert!(url.contains("state=a+state%2Fwith+symbols"));
    }

    #[test]
    fn reads_the_account_out_of_an_id_token() {
        let token = id_token(json!({
            "email": "person@example.com",
            "email_verified": true,
            "name": "A Person",
        }));

        let account = account_from(&token).expect("the token should parse");

        assert_eq!(account.email, "person@example.com");
        assert!(account.email_verified);
        assert_eq!(account.name, "A Person");
    }

    #[test]
    fn an_unverified_address_stays_unverified() {
        let token = id_token(json!({ "email": "person@example.com" }));

        let account = account_from(&token).expect("the token should parse");

        assert!(!account.email_verified);
    }

    #[test]
    fn a_token_that_is_not_a_token_is_an_error() {
        let err = account_from("not-a-token").expect_err("the token should be rejected");

        assert_eq!(err.app_code(), code::UNKNOWN);
    }
}
