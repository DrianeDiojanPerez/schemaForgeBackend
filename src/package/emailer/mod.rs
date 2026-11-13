use std::collections::HashMap;

use async_trait::async_trait;
use lettre::message::header::{ContentType, MIME_VERSION_1_0};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::{Mail, MailEncryption};

const TEMPLATES: &[(&str, &str)] = &[
    (
        "password-reset",
        include_str!("templates/password-reset.html"),
    ),
    (
        "schema-invitation",
        include_str!("templates/schema-invitation.html"),
    ),
    (
        "schema-shared",
        include_str!("templates/schema-shared.html"),
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum EmailerError {
    #[error("template `{0}` not found")]
    TemplateNotFound(String),
    #[error("failed to build the message: {0}")]
    Build(#[from] lettre::error::Error),
    #[error("invalid mail address: {0}")]
    Address(#[from] lettre::address::AddressError),
    #[error("failed to send the message: {0}")]
    Transport(#[from] lettre::transport::smtp::Error),
}

#[async_trait]
pub trait Emailer: Send + Sync {
    async fn send_html(
        &self,
        to: &str,
        subject: &str,
        template_name: &str,
        data: HashMap<String, String>,
    ) -> Result<(), EmailerError>;
}

pub struct SmtpEmailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: String,
}

impl SmtpEmailer {
    pub fn new(cfg: &Mail) -> Result<Self, EmailerError> {
        let mut builder = match cfg.encryption {
            MailEncryption::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host)
            }
            MailEncryption::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)?
            }
            MailEncryption::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)?,
        }
        .port(cfg.port);

        if !cfg.username.is_empty() {
            builder =
                builder.credentials(Credentials::new(cfg.username.clone(), cfg.password.clone()));
        }

        Ok(Self {
            transport: builder.build(),
            from: format!("{} <{}>", cfg.from_name, cfg.from_address),
        })
    }

    fn render(template_name: &str, data: &HashMap<String, String>) -> Result<String, EmailerError> {
        let template = TEMPLATES
            .iter()
            .find(|(name, _)| *name == template_name)
            .map(|(_, body)| *body)
            .ok_or_else(|| EmailerError::TemplateNotFound(template_name.to_owned()))?;

        Ok(data.iter().fold(template.to_owned(), |body, (key, value)| {
            body.replace(&format!("{{{{{key}}}}}"), value)
        }))
    }

    // Gmail takes a mail without these two headers and then quietly drops
    // it, so the sender sees a success and the inbox never sees the mail.
    fn message(from: &str, to: &str, subject: &str, body: String) -> Result<Message, EmailerError> {
        Ok(Message::builder()
            .from(from.parse()?)
            .to(to.parse()?)
            .subject(subject)
            .message_id(None)
            .header(MIME_VERSION_1_0)
            .header(ContentType::TEXT_HTML)
            .body(body)?)
    }
}

#[async_trait]
impl Emailer for SmtpEmailer {
    async fn send_html(
        &self,
        to: &str,
        subject: &str,
        template_name: &str,
        data: HashMap<String, String>,
    ) -> Result<(), EmailerError> {
        let body = Self::render(template_name, &data)?;

        self.transport
            .send(Self::message(&self.from, to, subject, body)?)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_template_placeholders() {
        let data = HashMap::from([
            ("username".to_owned(), "admin".to_owned()),
            (
                "callbackURI".to_owned(),
                "https://example.com/reset?token=abc".to_owned(),
            ),
        ]);

        let body = SmtpEmailer::render("password-reset", &data).expect("template should render");

        assert!(body.contains("Hey admin,"));
        assert!(body.contains("https://example.com/reset?token=abc"));
        assert!(!body.contains("{{"));
    }

    #[test]
    fn an_invitation_names_who_shared_what_and_where_to_go() {
        let data = HashMap::from([
            ("inviter".to_owned(), "Ada Lovelace".to_owned()),
            ("avatar".to_owned(), "AL".to_owned()),
            ("schemaName".to_owned(), "Course Registration".to_owned()),
            ("role".to_owned(), "editor".to_owned()),
            (
                "callbackURI".to_owned(),
                "https://example.com/invitations/abc".to_owned(),
            ),
        ]);

        let body = SmtpEmailer::render("schema-invitation", &data).expect("template should render");

        assert!(body.contains("Ada Lovelace invited you"));
        assert!(body.contains(">AL</td>"));
        assert!(body.contains(">Course Registration</strong>"));
        assert!(body.contains("with editor access."));
        assert!(body.contains("https://example.com/invitations/abc"));
        assert!(!body.contains("{{"));
    }

    #[test]
    fn a_shared_mail_says_what_they_can_do_now_and_where() {
        let data = HashMap::from([
            ("inviter".to_owned(), "Ada Lovelace".to_owned()),
            ("avatar".to_owned(), "AL".to_owned()),
            ("schemaName".to_owned(), "Course Registration".to_owned()),
            ("verb".to_owned(), "edit".to_owned()),
            ("appURI".to_owned(), "https://example.com/".to_owned()),
        ]);

        let body = SmtpEmailer::render("schema-shared", &data).expect("template should render");

        assert!(body.contains("Ada Lovelace shared a schema with you"));
        assert!(body.contains("You can now edit <strong"));
        assert!(body.contains("href=\"https://example.com/\""));
        assert!(!body.contains("{{"));
    }

    #[test]
    fn a_message_carries_the_headers_gmail_drops_it_without() {
        let message = SmtpEmailer::message(
            "SchemaForge <noreply@example.com>",
            "sam@example.com",
            "Hello",
            "<p>hi</p>".to_owned(),
        )
        .expect("the message should build");

        let raw = String::from_utf8(message.formatted()).expect("the message should be text");

        assert!(raw.contains("MIME-Version: 1.0\r\n"), "{raw}");
        assert!(raw.contains("Message-ID: <"), "{raw}");
        assert!(raw.contains("Content-Type: text/html; charset=utf-8\r\n"));
    }

    #[test]
    fn a_message_to_an_address_that_is_not_one_is_refused() {
        assert!(SmtpEmailer::message(
            "SchemaForge <noreply@example.com>",
            "not an address",
            "Hello",
            String::new(),
        )
        .is_err());
    }

    #[test]
    fn reports_an_unknown_template() {
        assert!(SmtpEmailer::render("nope", &HashMap::new()).is_err());
    }
}
