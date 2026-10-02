use std::fmt;

use crate::package::masked::MaskedString;

#[derive(Clone)]
pub struct Google {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

impl fmt::Debug for Google {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Google")
            .field("client_id", &self.client_id)
            .field("client_secret", &MaskedString)
            .field("redirect_uri", &self.redirect_uri)
            .finish()
    }
}
