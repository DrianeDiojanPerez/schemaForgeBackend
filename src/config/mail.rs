use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MailEncryption {
    #[default]
    None,
    StartTls,
    Tls,
}

impl FromStr for MailEncryption {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(MailEncryption::None),
            "starttls" => Ok(MailEncryption::StartTls),
            "tls" => Ok(MailEncryption::Tls),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Mail {
    pub host: String,
    pub port: u16,
    pub encryption: MailEncryption,
    pub username: String,
    pub password: String,
    pub from_address: String,
    pub from_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_encryption_name_in_any_case() {
        assert_eq!("none".parse(), Ok(MailEncryption::None));
        assert_eq!(" StartTLS ".parse(), Ok(MailEncryption::StartTls));
        assert_eq!("TLS".parse(), Ok(MailEncryption::Tls));
    }

    #[test]
    fn rejects_anything_that_is_not_an_encryption() {
        assert_eq!("ssl".parse::<MailEncryption>(), Err(()));
        assert_eq!("".parse::<MailEncryption>(), Err(()));
    }

    #[test]
    fn defaults_to_none_for_the_local_mail_catcher() {
        assert_eq!(MailEncryption::default(), MailEncryption::None);
    }
}
