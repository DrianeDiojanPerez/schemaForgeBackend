use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::package::auth::{Auth, GoogleAccount, GoogleIdentity, Store};
use crate::package::auth::{AuthenticationTokens, Identity};
use crate::package::crypto;
use crate::package::emailer::Emailer;
use crate::package::errdef::Error;
use crate::package::jwt::{Claims, TokenGenerator};

const PASSWORD_RESET_TTL_MINUTES: i64 = 15;

const INVALID_CREDENTIALS: &str = "invalid username or password";
const INVALID_REFRESH_TOKEN: &str = "invalid or malformed refresh token";
const UNVERIFIED_GOOGLE_EMAIL: &str = "the google account has no verified email address";
const GOOGLE_ACCOUNT_NOT_ALLOWED: &str = "this google account is not allowed to use the app";

/// Google hands back one name. Everything after the first space is the last
/// name, so a person with one word for a name keeps an empty last name.
fn split_name(name: &str) -> (String, String) {
    match name.trim().split_once(' ') {
        Some((first, last)) => (first.to_owned(), last.trim().to_owned()),
        None => (name.trim().to_owned(), String::new()),
    }
}

pub struct AuthService {
    jwt: Arc<dyn TokenGenerator>,
    store: Arc<dyn Store>,
    mailer: Arc<dyn Emailer>,
    google: Arc<dyn GoogleIdentity>,
    token_ttl: i64,
    refresh_token_ttl: i64,
}

impl AuthService {
    pub fn new(
        jwt: Arc<dyn TokenGenerator>,
        store: Arc<dyn Store>,
        mailer: Arc<dyn Emailer>,
        google: Arc<dyn GoogleIdentity>,
        token_ttl: i64,
        refresh_token_ttl: i64,
    ) -> Self {
        Self {
            jwt,
            store,
            mailer,
            google,
            token_ttl,
            refresh_token_ttl,
        }
    }

    fn generate_tokens(&self, user: &Identity) -> Result<AuthenticationTokens, Error> {
        let now = Utc::now();

        let access_claims = Claims::from([
            ("user_id".to_owned(), Value::from(user.id.to_string())),
            ("roles".to_owned(), Value::from(user.roles.clone())),
        ]);

        let token = self
            .jwt
            .generate_token(
                access_claims,
                (now + Duration::seconds(self.token_ttl)).timestamp(),
            )
            .map_err(Error::unknown)?;

        let refresh_claims =
            Claims::from([("user_id".to_owned(), Value::from(user.id.to_string()))]);

        let refresh_token = self
            .jwt
            .generate_token(
                refresh_claims,
                (now + Duration::seconds(self.refresh_token_ttl)).timestamp(),
            )
            .map_err(Error::unknown)?;

        Ok(AuthenticationTokens {
            token,
            refresh_token,
        })
    }

    fn user_id_from(&self, token: &str) -> Result<Uuid, Error> {
        let claims = self
            .jwt
            .validate_token(token)
            .map_err(|err| Error::unauthorized(INVALID_REFRESH_TOKEN).with_cause(err))?;

        claims
            .get("user_id")
            .and_then(Value::as_str)
            .and_then(|raw| Uuid::parse_str(raw).ok())
            .ok_or_else(|| Error::unauthorized(INVALID_REFRESH_TOKEN))
    }

    /// Google owns the name and the photo, so the record is brought back in
    /// line on every sign in. The write only happens on a real difference,
    /// which keeps a repeat sign in from touching the row at all.
    async fn follow_google_profile(
        &self,
        user: &Identity,
        account: &GoogleAccount,
    ) -> Result<(), Error> {
        let (first_name, last_name) = split_name(&account.name);
        let avatar_url = account.picture.as_deref();

        if user.first_name == first_name
            && user.last_name == last_name
            && user.avatar_url.as_deref() == avatar_url
        {
            return Ok(());
        }

        self.store
            .update_profile(user.id, &first_name, &last_name, avatar_url)
            .await
            .map_err(Error::unknown)
    }

    async fn require_user_by_id(&self, user_id: Uuid) -> Result<Identity, Error> {
        self.store
            .find_user_by_id(user_id)
            .await
            .map_err(Error::unknown)?
            .ok_or_else(|| Error::unauthorized(INVALID_REFRESH_TOKEN))
    }
}

#[async_trait]
impl Auth for AuthService {
    async fn generate_token(
        &self,
        email: &str,
        password: &str,
    ) -> Result<AuthenticationTokens, Error> {
        tracing::debug!(email, "Searching for user with email");

        let user = self
            .store
            .find_user_by_email(email)
            .await
            .map_err(Error::unknown)?
            .ok_or_else(|| Error::unauthorized(INVALID_CREDENTIALS))?;

        if crypto::compare_hash_and_password(&user.password, password).is_err() {
            tracing::debug!("Password Comparison Failed");
            return Err(Error::unauthorized(INVALID_CREDENTIALS));
        }

        self.generate_tokens(&user)
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthenticationTokens, Error> {
        let user_id = self.user_id_from(refresh_token)?;
        let user = self.require_user_by_id(user_id).await?;

        self.generate_tokens(&user)
    }

    fn google_login_url(&self, state: &str) -> String {
        self.google.consent_url(state)
    }

    async fn login_with_google(&self, code: &str) -> Result<AuthenticationTokens, Error> {
        let account = self.google.account_for(code).await?;

        if !account.email_verified {
            return Err(Error::unauthorized(UNVERIFIED_GOOGLE_EMAIL));
        }

        // Google says who the person is, the user table says whether they may
        // be here. Signing in never creates an account.
        let user = self
            .store
            .find_user_by_email(&account.email)
            .await
            .map_err(Error::unknown)?
            .ok_or_else(|| Error::forbidden(GOOGLE_ACCOUNT_NOT_ALLOWED))?;

        self.follow_google_profile(&user, &account).await?;

        self.generate_tokens(&user)
    }

    async fn get_identity(&self, access_token: &str) -> Result<Identity, Error> {
        let user_id = self.user_id_from(access_token)?;

        self.require_user_by_id(user_id).await
    }

    async fn password_recovery(&self, email: &str, callback_uri: &str) -> Result<(), Error> {
        let user = self
            .store
            .find_user_by_email(email)
            .await
            .map_err(Error::unknown)?
            .ok_or_else(|| Error::not_found("invalid email address"))?;

        // Only one pending request per address is kept.
        self.store
            .delete_password_reset(email)
            .await
            .map_err(Error::unknown)?;

        let token = crypto::random_token();

        // Only the digest is stored, the raw token travels by email.
        self.store
            .create_password_reset(email, &crypto::hash_token(&token))
            .await
            .map_err(Error::unknown)?;

        let data = HashMap::from([
            ("username".to_owned(), user.user_name.clone()),
            ("callbackURI".to_owned(), format!("{callback_uri}{token}")),
        ]);

        self.mailer
            .send_html(&user.email, "Password Recovery", "password-reset", data)
            .await
            .map_err(Error::unknown)?;

        Ok(())
    }

    async fn reset_password(&self, token: &str, new_password: &str) -> Result<(), Error> {
        let password_reset = self
            .store
            .find_password_by_token(&crypto::hash_token(token))
            .await
            .map_err(Error::unknown)?
            .ok_or_else(|| Error::bad_request("invalid or expired token"))?;

        if password_reset.created_at + Duration::minutes(PASSWORD_RESET_TTL_MINUTES) < Utc::now() {
            return Err(Error::bad_request("invalid or expired token"));
        }

        let hashed_password = crypto::hash_password(new_password)?;

        self.store
            .reset_password(&password_reset.email, &hashed_password)
            .await
            .map_err(Error::unknown)?;

        self.store
            .delete_password_reset(&password_reset.email)
            .await
            .map_err(Error::unknown)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::sync::Mutex;

    use async_trait::async_trait;

    use crate::package::auth::{GoogleAccount, PasswordReset};
    use crate::package::emailer::EmailerError;
    use crate::package::errdef::code;
    use crate::package::jwt::HmacTokenGenerator;
    use crate::package::masked::MaskedBytes;

    const PASSWORD: &str = "Sup3r$ecret";

    /// user, first name, last name, avatar url.
    type ProfileUpdate = (Uuid, String, String, Option<String>);

    #[derive(Default)]
    struct FakeStore {
        users: Vec<Identity>,
        resets: Mutex<Vec<PasswordReset>>,
        password_updates: Mutex<Vec<(String, String)>>,
        profile_updates: Mutex<Vec<ProfileUpdate>>,
        deleted_resets: Mutex<Vec<String>>,
    }

    impl FakeStore {
        fn with_user(user: Identity) -> Self {
            Self {
                users: vec![user],
                ..Self::default()
            }
        }

        fn push_reset(&self, email: &str, token: &str, created_at: chrono::DateTime<Utc>) {
            self.resets.lock().unwrap().push(PasswordReset {
                email: email.to_owned(),
                token: token.to_owned(),
                created_at,
            });
        }
    }

    #[async_trait]
    impl Store for FakeStore {
        async fn find_user_by_id(&self, user_id: Uuid) -> Result<Option<Identity>, sqlx::Error> {
            Ok(self.users.iter().find(|u| u.id == user_id).cloned())
        }

        async fn find_user_by_email(&self, email: &str) -> Result<Option<Identity>, sqlx::Error> {
            Ok(self.users.iter().find(|u| u.email == email).cloned())
        }

        async fn update_profile(
            &self,
            user_id: Uuid,
            first_name: &str,
            last_name: &str,
            avatar_url: Option<&str>,
        ) -> Result<(), sqlx::Error> {
            self.profile_updates.lock().unwrap().push((
                user_id,
                first_name.to_owned(),
                last_name.to_owned(),
                avatar_url.map(ToOwned::to_owned),
            ));
            Ok(())
        }

        async fn create_password_reset(&self, email: &str, token: &str) -> Result<(), sqlx::Error> {
            self.push_reset(email, token, Utc::now());
            Ok(())
        }

        async fn reset_password(&self, email: &str, new_password: &str) -> Result<(), sqlx::Error> {
            self.password_updates
                .lock()
                .unwrap()
                .push((email.to_owned(), new_password.to_owned()));
            Ok(())
        }

        async fn find_password_by_token(
            &self,
            token: &str,
        ) -> Result<Option<PasswordReset>, sqlx::Error> {
            Ok(self
                .resets
                .lock()
                .unwrap()
                .iter()
                .find(|reset| reset.token == token)
                .cloned())
        }

        async fn delete_password_reset(&self, email: &str) -> Result<(), sqlx::Error> {
            self.deleted_resets.lock().unwrap().push(email.to_owned());
            self.resets.lock().unwrap().retain(|r| r.email != email);
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeGoogle {
        account: Option<GoogleAccount>,
        reachable: bool,
    }

    impl FakeGoogle {
        fn answering_with(email: &str, email_verified: bool) -> Self {
            Self::answering_as(
                email,
                email_verified,
                "A Person",
                Some("https://photo.test/a"),
            )
        }

        fn answering_as(
            email: &str,
            email_verified: bool,
            name: &str,
            picture: Option<&str>,
        ) -> Self {
            Self {
                account: Some(GoogleAccount {
                    email: email.to_owned(),
                    email_verified,
                    name: name.to_owned(),
                    picture: picture.map(ToOwned::to_owned),
                }),
                reachable: true,
            }
        }

        fn down() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl GoogleIdentity for FakeGoogle {
        fn consent_url(&self, state: &str) -> String {
            format!("https://accounts.google.test/consent?state={state}")
        }

        async fn account_for(&self, _code: &str) -> Result<GoogleAccount, Error> {
            if !self.reachable {
                return Err(Error::unavailable("google could not be reached"));
            }

            self.account
                .clone()
                .ok_or_else(|| Error::unauthorized("google rejected the sign in code"))
        }
    }

    /// to, subject, template name, template data.
    type SentMail = (String, String, String, HashMap<String, String>);

    #[derive(Default)]
    struct FakeMailer {
        sent: Mutex<Vec<SentMail>>,
    }

    #[async_trait]
    impl Emailer for FakeMailer {
        async fn send_html(
            &self,
            to: &str,
            subject: &str,
            template_name: &str,
            data: HashMap<String, String>,
        ) -> Result<(), EmailerError> {
            self.sent.lock().unwrap().push((
                to.to_owned(),
                subject.to_owned(),
                template_name.to_owned(),
                data,
            ));
            Ok(())
        }
    }

    fn a_user() -> Identity {
        Identity {
            id: Uuid::new_v4(),
            email: "admin@example.com".to_owned(),
            user_name: "admin".to_owned(),
            first_name: "App".to_owned(),
            last_name: "Admin".to_owned(),
            avatar_url: None,
            password: crypto::hash_password(PASSWORD).expect("hashing should succeed"),
            roles: vec!["Admin".to_owned()],
        }
    }

    fn service_with(
        store: Arc<FakeStore>,
        mailer: Arc<FakeMailer>,
    ) -> (AuthService, Arc<dyn TokenGenerator>) {
        service_with_google(store, mailer, Arc::new(FakeGoogle::default()))
    }

    fn service_with_google(
        store: Arc<FakeStore>,
        mailer: Arc<FakeMailer>,
        google: Arc<FakeGoogle>,
    ) -> (AuthService, Arc<dyn TokenGenerator>) {
        let jwt: Arc<dyn TokenGenerator> =
            Arc::new(HmacTokenGenerator::new(&MaskedBytes::new("secret")));

        (
            AuthService::new(jwt.clone(), store, mailer, google, 3600, 604_800),
            jwt,
        )
    }

    fn code_of(err: &Error) -> i32 {
        match err {
            Error::App(err) => err.code,
            Error::Validation(_) => code::VALIDATION_FAILED,
        }
    }

    #[tokio::test]
    async fn login_returns_a_token_pair() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let (service, jwt) = service_with(store, Arc::new(FakeMailer::default()));

        let tokens = service
            .generate_token(&user.email, PASSWORD)
            .await
            .expect("login should succeed");

        let claims = jwt
            .validate_token(&tokens.token)
            .expect("the access token should validate");

        assert_eq!(
            claims.get("user_id").and_then(Value::as_str),
            Some(user.id.to_string().as_str())
        );
        assert_eq!(
            claims.get("roles"),
            Some(&Value::from(vec!["Admin".to_owned()]))
        );

        // The refresh token carries the identity only, never the roles.
        let refresh_claims = jwt
            .validate_token(&tokens.refresh_token)
            .expect("the refresh token should validate");
        assert!(!refresh_claims.contains_key("roles"));
    }

    #[tokio::test]
    async fn login_rejects_a_wrong_password() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let (service, _) = service_with(store, Arc::new(FakeMailer::default()));

        let err = service
            .generate_token(&user.email, "not the password")
            .await
            .expect_err("login should fail");

        assert_eq!(code_of(&err), code::UNAUTHORIZED);
        assert!(err.to_string().contains(INVALID_CREDENTIALS));
    }

    #[tokio::test]
    async fn login_does_not_reveal_whether_the_email_exists() {
        let store = Arc::new(FakeStore::with_user(a_user()));
        let (service, _) = service_with(store, Arc::new(FakeMailer::default()));

        let unknown = service
            .generate_token("nobody@example.com", PASSWORD)
            .await
            .expect_err("login should fail");
        let wrong_password = service
            .generate_token("admin@example.com", "wrong")
            .await
            .expect_err("login should fail");

        assert_eq!(unknown.to_string(), wrong_password.to_string());
    }

    #[tokio::test]
    async fn refresh_issues_a_new_pair() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let (service, _) = service_with(store, Arc::new(FakeMailer::default()));

        let tokens = service
            .generate_token(&user.email, PASSWORD)
            .await
            .expect("login should succeed");

        let refreshed = service
            .refresh_token(&tokens.refresh_token)
            .await
            .expect("refresh should succeed");

        assert!(!refreshed.token.is_empty());
        assert!(!refreshed.refresh_token.is_empty());
    }

    #[tokio::test]
    async fn refresh_rejects_a_malformed_token() {
        let store = Arc::new(FakeStore::with_user(a_user()));
        let (service, _) = service_with(store, Arc::new(FakeMailer::default()));

        let err = service
            .refresh_token("not.a.token")
            .await
            .expect_err("refresh should fail");

        assert_eq!(code_of(&err), code::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn refresh_rejects_a_token_for_a_deleted_user() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let (service, jwt) = service_with(store, Arc::new(FakeMailer::default()));

        let orphan = jwt
            .generate_token(
                Claims::from([(
                    "user_id".to_owned(),
                    Value::from(Uuid::new_v4().to_string()),
                )]),
                Utc::now().timestamp() + 3600,
            )
            .expect("token should be signed");

        let err = service
            .refresh_token(&orphan)
            .await
            .expect_err("refresh should fail");

        assert_eq!(code_of(&err), code::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_consent_url_round_trips_the_state_untouched() {
        let store = Arc::new(FakeStore::with_user(a_user()));
        let (service, _) = service_with(store, Arc::new(FakeMailer::default()));

        assert!(service
            .google_login_url("opaque-state")
            .ends_with("opaque-state"));
    }

    #[tokio::test]
    async fn a_google_account_that_matches_a_user_gets_the_same_token_pair() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let google = Arc::new(FakeGoogle::answering_with(&user.email, true));
        let (service, jwt) = service_with_google(store, Arc::new(FakeMailer::default()), google);

        let tokens = service
            .login_with_google("a-code")
            .await
            .expect("the sign in should succeed");

        let claims = jwt
            .validate_token(&tokens.token)
            .expect("the access token should validate");

        assert_eq!(
            claims.get("user_id").and_then(Value::as_str),
            Some(user.id.to_string().as_str())
        );
    }

    #[tokio::test]
    async fn the_record_follows_the_google_name_and_photo() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let google = Arc::new(FakeGoogle::answering_as(
            &user.email,
            true,
            "Ada Byron Lovelace",
            Some("https://photo.test/ada"),
        ));
        let (service, _) =
            service_with_google(store.clone(), Arc::new(FakeMailer::default()), google);

        service
            .login_with_google("a-code")
            .await
            .expect("the sign in should succeed");

        let updates = store.profile_updates.lock().unwrap();
        let (id, first_name, last_name, avatar_url) =
            updates.first().expect("the profile should be written");

        assert_eq!(id, &user.id);
        assert_eq!(first_name, "Ada");
        assert_eq!(last_name, "Byron Lovelace", "only the first space splits");
        assert_eq!(avatar_url.as_deref(), Some("https://photo.test/ada"));
    }

    #[tokio::test]
    async fn a_one_word_google_name_leaves_the_last_name_empty() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let google = Arc::new(FakeGoogle::answering_as(&user.email, true, "Prince", None));
        let (service, _) =
            service_with_google(store.clone(), Arc::new(FakeMailer::default()), google);

        service
            .login_with_google("a-code")
            .await
            .expect("the sign in should succeed");

        let updates = store.profile_updates.lock().unwrap();
        let (_, first_name, last_name, avatar_url) =
            updates.first().expect("the profile should be written");

        assert_eq!(first_name, "Prince");
        assert!(last_name.is_empty());
        assert_eq!(avatar_url, &None);
    }

    #[tokio::test]
    async fn a_profile_that_already_matches_is_left_alone() {
        let mut user = a_user();
        user.first_name = "A".to_owned();
        user.last_name = "Person".to_owned();
        user.avatar_url = Some("https://photo.test/a".to_owned());

        let store = Arc::new(FakeStore::with_user(user.clone()));
        let google = Arc::new(FakeGoogle::answering_with(&user.email, true));
        let (service, _) =
            service_with_google(store.clone(), Arc::new(FakeMailer::default()), google);

        service
            .login_with_google("a-code")
            .await
            .expect("the sign in should succeed");

        assert!(store.profile_updates.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_google_account_with_no_user_is_refused() {
        let store = Arc::new(FakeStore::with_user(a_user()));
        let google = Arc::new(FakeGoogle::answering_with("stranger@example.com", true));
        let (service, _) = service_with_google(store, Arc::new(FakeMailer::default()), google);

        let err = service
            .login_with_google("a-code")
            .await
            .expect_err("the sign in should fail");

        assert_eq!(code_of(&err), code::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_unverified_google_address_never_reaches_the_user_table() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let google = Arc::new(FakeGoogle::answering_with(&user.email, false));
        let (service, _) = service_with_google(store, Arc::new(FakeMailer::default()), google);

        let err = service
            .login_with_google("a-code")
            .await
            .expect_err("the sign in should fail");

        assert_eq!(code_of(&err), code::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn google_being_down_is_not_the_users_fault() {
        let store = Arc::new(FakeStore::with_user(a_user()));
        let (service, _) = service_with_google(
            store,
            Arc::new(FakeMailer::default()),
            Arc::new(FakeGoogle::down()),
        );

        let err = service
            .login_with_google("a-code")
            .await
            .expect_err("the sign in should fail");

        assert_eq!(code_of(&err), code::UNAVAILABLE);
    }

    #[tokio::test]
    async fn get_identity_resolves_the_user_behind_a_token() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let (service, _) = service_with(store, Arc::new(FakeMailer::default()));

        let tokens = service
            .generate_token(&user.email, PASSWORD)
            .await
            .expect("login should succeed");

        let identity = service
            .get_identity(&tokens.token)
            .await
            .expect("identity should resolve");

        assert_eq!(identity.id, user.id);
        assert_eq!(identity.roles, vec!["Admin".to_owned()]);
    }

    #[tokio::test]
    async fn password_recovery_stores_a_digest_and_mails_the_raw_token() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let mailer = Arc::new(FakeMailer::default());
        let (service, _) = service_with(store.clone(), mailer.clone());

        service
            .password_recovery(&user.email, "https://example.com/reset?token=")
            .await
            .expect("recovery should succeed");

        let sent = mailer.sent.lock().unwrap();
        let (to, subject, template, data) = sent.first().expect("a mail should have been sent");

        assert_eq!(to, &user.email);
        assert_eq!(subject, "Password Recovery");
        assert_eq!(template, "password-reset");
        assert_eq!(data.get("username"), Some(&user.user_name));

        let raw_token = data
            .get("callbackURI")
            .and_then(|uri| uri.split("token=").nth(1))
            .expect("the callback should carry the token");

        let stored = store.resets.lock().unwrap();
        let reset = stored.first().expect("a reset should be stored");

        assert_ne!(reset.token, raw_token, "the raw token must not be stored");
        assert_eq!(reset.token, crypto::hash_token(raw_token));
    }

    #[tokio::test]
    async fn password_recovery_clears_a_previous_request() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let (service, _) = service_with(store.clone(), Arc::new(FakeMailer::default()));

        store.push_reset(&user.email, "stale", Utc::now());

        service
            .password_recovery(&user.email, "https://example.com/reset?token=")
            .await
            .expect("recovery should succeed");

        assert_eq!(store.deleted_resets.lock().unwrap().len(), 1);
        assert_eq!(store.resets.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn password_recovery_reports_an_unknown_address() {
        let store = Arc::new(FakeStore::with_user(a_user()));
        let mailer = Arc::new(FakeMailer::default());
        let (service, _) = service_with(store, mailer.clone());

        let err = service
            .password_recovery("nobody@example.com", "https://example.com/")
            .await
            .expect_err("recovery should fail");

        assert_eq!(code_of(&err), code::NOT_FOUND);
        assert!(mailer.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reset_password_updates_the_hash_and_consumes_the_token() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let (service, _) = service_with(store.clone(), Arc::new(FakeMailer::default()));

        store.push_reset(&user.email, &crypto::hash_token("raw-token"), Utc::now());

        service
            .reset_password("raw-token", "N3wP@ssword")
            .await
            .expect("reset should succeed");

        let updates = store.password_updates.lock().unwrap();
        let (email, hash) = updates.first().expect("the password should be updated");

        assert_eq!(email, &user.email);
        assert!(crypto::compare_hash_and_password(hash, "N3wP@ssword").is_ok());
        assert!(store.resets.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reset_password_rejects_an_expired_token() {
        let user = a_user();
        let store = Arc::new(FakeStore::with_user(user.clone()));
        let (service, _) = service_with(store.clone(), Arc::new(FakeMailer::default()));

        store.push_reset(
            &user.email,
            &crypto::hash_token("raw-token"),
            Utc::now() - Duration::minutes(PASSWORD_RESET_TTL_MINUTES + 1),
        );

        let err = service
            .reset_password("raw-token", "N3wP@ssword")
            .await
            .expect_err("reset should fail");

        assert_eq!(code_of(&err), code::BAD_REQUEST);
        assert!(store.password_updates.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn reset_password_rejects_an_unknown_token() {
        let store = Arc::new(FakeStore::with_user(a_user()));
        let (service, _) = service_with(store.clone(), Arc::new(FakeMailer::default()));

        let err = service
            .reset_password("never-issued", "N3wP@ssword")
            .await
            .expect_err("reset should fail");

        assert_eq!(code_of(&err), code::BAD_REQUEST);
        assert!(store.password_updates.lock().unwrap().is_empty());
    }
}
