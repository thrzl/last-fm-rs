use url::Url;

const DEFAULT_API_BASE: &str = "https://ws.audioscrobbler.com/2.0/";
const DEFAULT_AUTH_URL: &str = "http://www.last.fm/api/auth/";

/// Authentication mode for the client
#[derive(Debug, Clone)]
pub(crate) enum AuthMode {
    /// Last.fm API authentication with API key, secret, and optional session key
    LastFm {
        api_key: String,
        api_secret: String,
        session_key: Option<String>,
        base_url: Url,
        auth_url: Url,
    },
    /// Token-based authentication for custom scrobble servers
    Token { base_url: Url, token: String },
}

impl AuthMode {
    /// Create a new Last.fm auth mode
    pub fn lastfm(api_key: impl Into<String>, api_secret: impl Into<String>) -> Self {
        Self::LastFm {
            api_key: api_key.into(),
            api_secret: api_secret.into(),
            session_key: None,
            base_url: Url::parse(DEFAULT_API_BASE).expect("invalid default URL"),
            auth_url: Url::parse(DEFAULT_AUTH_URL).expect("invalid default URL"),
        }
    }

    /// Create a new token-based auth mode
    pub fn token(base_url: Url, token: impl Into<String>) -> Self {
        Self::Token {
            base_url,
            token: token.into(),
        }
    }

    /// Set session key for Last.fm mode (no-op for Token mode)
    pub fn set_session_key(&mut self, key: impl Into<String>) {
        if let Self::LastFm { session_key, .. } = self {
            *session_key = Some(key.into());
        }
    }

    /// Override the API base URL (Last.fm mode only, no-op for Token mode)
    pub fn set_api_base(&mut self, url: Url) {
        if let Self::LastFm { base_url, .. } = self {
            *base_url = url;
        }
    }

    /// Override the authorization URL (Last.fm mode only, no-op for Token mode)
    pub fn set_auth_url(&mut self, url: Url) {
        if let Self::LastFm { auth_url, .. } = self {
            *auth_url = url;
        }
    }

    /// Get session key for Last.fm mode (None for Token mode)
    #[cfg(test)]
    pub(crate) fn session_key(&self) -> Option<&str> {
        match self {
            Self::LastFm { session_key, .. } => session_key.as_deref(),
            Self::Token { .. } => None,
        }
    }

    /// Get API key (Last.fm mode only)
    #[cfg(test)]
    pub(crate) fn api_key(&self) -> Option<&str> {
        match self {
            Self::LastFm { api_key, .. } => Some(api_key),
            Self::Token { .. } => None,
        }
    }

    /// Get API secret (Last.fm mode only)
    #[cfg(test)]
    pub(crate) fn api_secret(&self) -> Option<&str> {
        match self {
            Self::LastFm { api_secret, .. } => Some(api_secret),
            Self::Token { .. } => None,
        }
    }

    /// Check if this is Last.fm mode
    #[cfg(test)]
    pub(crate) fn is_lastfm(&self) -> bool {
        matches!(self, Self::LastFm { .. })
    }

    /// Check if this is token mode
    #[cfg(test)]
    pub(crate) fn is_token(&self) -> bool {
        matches!(self, Self::Token { .. })
    }
}
