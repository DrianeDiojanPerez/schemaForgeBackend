use std::collections::HashMap;

use async_trait::async_trait;
use lettre::message::MultiPart;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::{Mail, MailEncryption};

struct Template {
    name: &'static str,
    html: &'static str,
    text: &'static str,
}

const TEMPLATES: &[Template] = &[
    Template {
        name: "password-reset",
        html: include_str!("templates/password-reset.html"),
        text: include_str!("templates/password-reset.txt"),
    },
    Template {
        name: "schema-invitation",
        html: include_str!("templates/schema-invitation.html"),
        text: include_str!("templates/schema-invitation.txt"),
    },
    Template {
        name: "schema-shared",
        html: include_str!("templates/schema-shared.html"),
        text: include_str!("templates/schema-shared.txt"),
    },
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

    fn template(name: &str) -> Result<&'static Template, EmailerError> {
        TEMPLATES
            .iter()
            .find(|template| template.name == name)
            .ok_or_else(|| EmailerError::TemplateNotFound(name.to_owned()))
    }

    fn fill(body: &str, data: &HashMap<String, String>) -> String {
        data.iter().fold(body.to_owned(), |body, (key, value)| {
            body.replace(&format!("{{{{{key}}}}}"), value)
        })
    }

    fn render(template_name: &str, data: &HashMap<String, String>) -> Result<String, EmailerError> {
        Ok(Self::fill(Self::template(template_name)?.html, data))
    }

    fn render_text(
        template_name: &str,
        data: &HashMap<String, String>,
    ) -> Result<String, EmailerError> {
        Ok(Self::fill(Self::template(template_name)?.text, data))
    }

    // Gmail takes a mail with no MIME version or message id and quietly
    // drops it, and it counts a mail with no plain text part as spam, so
    // every mail goes out with both headers and both parts.
    fn message(
        from: &str,
        to: &str,
        subject: &str,
        text: String,
        html: String,
    ) -> Result<Message, EmailerError> {
        Ok(Message::builder()
            .from(from.parse()?)
            .to(to.parse()?)
            .subject(subject)
            .message_id(None)
            .multipart(MultiPart::alternative_plain_html(text, html))?)
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
        let html = Self::render(template_name, &data)?;
        let text = Self::render_text(template_name, &data)?;

        self.transport
            .send(Self::message(&self.from, to, subject, text, html)?)
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
            "hi in text".to_owned(),
            "<p>hi</p>".to_owned(),
        )
        .expect("the message should build");

        let raw = String::from_utf8(message.formatted()).expect("the message should be text");

        assert!(raw.contains("MIME-Version: 1.0\r\n"), "{raw}");
        assert!(raw.contains("Message-ID: <"), "{raw}");
        assert!(raw.contains("Content-Type: text/html; charset=utf-8\r\n"));
    }

    #[test]
    fn a_message_carries_a_plain_text_part_beside_the_html() {
        let message = SmtpEmailer::message(
            "SchemaForge <noreply@example.com>",
            "sam@example.com",
            "Hello",
            "hi in text".to_owned(),
            "<p>hi</p>".to_owned(),
        )
        .expect("the message should build");

        let raw = String::from_utf8(message.formatted()).expect("the message should be text");

        assert!(
            raw.contains("Content-Type: multipart/alternative;"),
            "{raw}"
        );
        assert!(raw.contains("Content-Type: text/plain; charset=utf-8\r\n"));
        assert!(raw.contains("hi in text"));
        assert!(raw.contains("<p>hi</p>"));
    }

    #[test]
    fn every_template_has_a_text_version_with_the_same_link_and_no_markup() {
        let data = HashMap::from([
            ("username".to_owned(), "admin".to_owned()),
            ("inviter".to_owned(), "Ada Lovelace".to_owned()),
            ("avatar".to_owned(), "<img src=\"x\">".to_owned()),
            ("schemaName".to_owned(), "Course Registration".to_owned()),
            ("role".to_owned(), "editor".to_owned()),
            ("verb".to_owned(), "edit".to_owned()),
            ("appURI".to_owned(), "https://example.com/".to_owned()),
            (
                "callbackURI".to_owned(),
                "https://example.com/go".to_owned(),
            ),
        ]);

        for template in TEMPLATES {
            let text = SmtpEmailer::render_text(template.name, &data)
                .expect("the text version should render");

            assert!(text.contains("https://example.com/"), "{}", template.name);
            assert!(!text.contains("{{"), "{}: {text}", template.name);
            assert!(!text.contains('<'), "{}: {text}", template.name);
        }
    }

    #[test]
    fn a_message_to_an_address_that_is_not_one_is_refused() {
        assert!(SmtpEmailer::message(
            "SchemaForge <noreply@example.com>",
            "not an address",
            "Hello",
            String::new(),
            String::new(),
        )
        .is_err());
    }

    #[test]
    fn reports_an_unknown_template() {
        assert!(SmtpEmailer::render("nope", &HashMap::new()).is_err());
    }
}
