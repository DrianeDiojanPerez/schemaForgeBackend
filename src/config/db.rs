use crate::package::masked::MaskedString;

#[derive(Clone)]
pub struct Db {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub ssl_mode: String,
    pub max_connections: u32,
    pub run_migrations: bool,
}

impl Db {
    /// Connection string handed to sqlx. The password is only materialised here.
    pub fn to_url(&self) -> String {
        format!(
            "postgres://{}:{}@{}:{}/{}?sslmode={}",
            encode(&self.username),
            encode(&self.password),
            self.host,
            self.port,
            self.database,
            self.ssl_mode
        )
    }
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &MaskedString)
            .field("ssl_mode", &self.ssl_mode)
            .field("max_connections", &self.max_connections)
            .field("run_migrations", &self.run_migrations)
            .finish()
    }
}

fn encode(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '@' => "%40".to_owned(),
            ':' => "%3A".to_owned(),
            '/' => "%2F".to_owned(),
            '?' => "%3F".to_owned(),
            '#' => "%23".to_owned(),
            '[' => "%5B".to_owned(),
            ']' => "%5D".to_owned(),
            other => other.to_string(),
        })
        .collect()
}
