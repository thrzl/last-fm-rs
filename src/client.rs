use std::collections::BTreeMap;

use crate::auth::{AuthToken, SessionKey};
use crate::auth_mode::AuthMode;
use crate::error::{Error, Result};
use crate::scrobble::{NowPlaying, Scrobble, ScrobbleResponse};
use crate::signature;

/// Last.fm API client
pub struct Client {
    auth: AuthMode,
    http_client: reqwest::Client,
}

impl Client {
    /// Create a new Last.fm client
    pub fn new(api_key: impl Into<String>, secret: impl Into<String>) -> Self {
        Self {
            auth: AuthMode::lastfm(api_key, secret),
            http_client: reqwest::Client::new(),
        }
    }

    /// Set session key for authenticated requests
    pub fn with_session_key(mut self, session_key: impl Into<String>) -> Self {
        self.auth.set_session_key(session_key);
        self
    }

    /// Override the API base URL (last.fm mode only)
    ///
    /// Defaults to `https://ws.audioscrobbler.com/2.0/`, last.fm's base URL. Use this to point at a
    /// last.fm-compatible API server.
    pub fn with_api_base(mut self, base_url: impl AsRef<str>) -> Result<Self> {
        let url = url::Url::parse(base_url.as_ref())?;
        self.auth.set_api_base(url);
        Ok(self)
    }

    /// Override the authorization URL (Last.fm mode only)
    ///
    /// Defaults to `http://www.last.fm/api/auth/`. Use this if your last.fm-compatible
    /// server exposes a different web auth flow.
    pub fn with_auth_url(mut self, auth_url: impl AsRef<str>) -> Result<Self> {
        let url = url::Url::parse(auth_url.as_ref())?;
        self.auth.set_auth_url(url);
        Ok(self)
    }

    /// Create a client for token-based authentication with a custom server
    ///
    /// This mode bypasses Last.fm's authentication flow and instead uses:
    /// - A static bearer token for authentication
    /// - A custom base URL for your scrobble server
    /// - JSON request/response bodies instead of form-encoded parameters
    ///
    /// # Example
    ///
    /// ```no_run
    /// use last_fm_rs::Client;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = Client::with_token(
    ///   "https://scrob.example.com/api/",
    ///   "my-secret-token"
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_token(base_url: impl AsRef<str>, token: impl Into<String>) -> Result<Self> {
        let url = url::Url::parse(base_url.as_ref())?;
        Ok(Self {
            auth: AuthMode::token(url, token),
            http_client: reqwest::Client::new(),
        })
    }

    /// Step 1: Get authentication token (Last.fm mode only)
    pub async fn get_token(&self) -> Result<AuthToken> {
        let (api_key, secret, base_url) = match &self.auth {
            AuthMode::LastFm {
                api_key,
                api_secret,
                base_url,
                ..
            } => (api_key, api_secret, base_url),
            AuthMode::Token { .. } => {
                return Err(Error::Auth(
                    "get_token() is only available in Last.fm mode".to_string(),
                ))
            }
        };

        let mut params = BTreeMap::new();
        params.insert("method".to_string(), "auth.getToken".to_string());
        params.insert("api_key".to_string(), api_key.clone());

        let sig = signature::generate(&params, secret);
        params.insert("api_sig".to_string(), sig);
        params.insert("format".to_string(), "json".to_string());

        let resp = self
            .http_client
            .get(base_url.clone())
            .query(&params)
            .send()
            .await?
            .error_for_status()?;

        let json: serde_json::Value = resp.json().await?;

        if let Some(token) = json.get("token") {
            Ok(AuthToken {
                token: token.as_str().unwrap().to_string(),
            })
        } else if let Some(error) = json.get("error") {
            Err(Error::Api(error.to_string()))
        } else {
            Err(Error::Api("Unexpected response format".to_string()))
        }
    }

    /// Step 2: Generate authorization URL (Last.fm mode only)
    pub fn get_auth_url(&self, token: &AuthToken) -> Result<String> {
        let (api_key, auth_url) = match &self.auth {
            AuthMode::LastFm {
                api_key, auth_url, ..
            } => (api_key, auth_url),
            AuthMode::Token { .. } => {
                return Err(Error::Auth(
                    "get_auth_url() is only available in Last.fm mode".to_string(),
                ))
            }
        };

        let mut url = auth_url.clone();
        url.query_pairs_mut()
            .append_pair("api_key", api_key)
            .append_pair("token", &token.token);
        Ok(url.to_string())
    }

    /// Step 3: Exchange token for session key (Last.fm mode only)
    pub async fn get_session(&self, token: &AuthToken) -> Result<SessionKey> {
        let (api_key, secret, base_url) = match &self.auth {
            AuthMode::LastFm {
                api_key,
                api_secret,
                base_url,
                ..
            } => (api_key, api_secret, base_url),
            AuthMode::Token { .. } => {
                return Err(Error::Auth(
                    "get_session() is only available in Last.fm mode".to_string(),
                ))
            }
        };

        let mut params = BTreeMap::new();
        params.insert("method".to_string(), "auth.getSession".to_string());
        params.insert("api_key".to_string(), api_key.clone());
        params.insert("token".to_string(), token.token.clone());

        let sig = signature::generate(&params, secret);
        params.insert("api_sig".to_string(), sig);
        params.insert("format".to_string(), "json".to_string());

        let resp = self
            .http_client
            .get(base_url.clone())
            .query(&params)
            .send()
            .await?
            .error_for_status()?;

        let json: serde_json::Value = resp.json().await?;

        if let Some(session) = json.get("session") {
            Ok(SessionKey {
                key: session["key"].as_str().unwrap().to_string(),
                name: session["name"].as_str().unwrap().to_string(),
            })
        } else if let Some(error) = json.get("error") {
            Err(Error::Auth(error.to_string()))
        } else {
            Err(Error::Auth("Unexpected response format".to_string()))
        }
    }

    /// Update "Now Playing" status
    pub async fn update_now_playing(&self, now_playing: &NowPlaying) -> Result<()> {
        match &self.auth {
            AuthMode::LastFm {
                api_key,
                api_secret,
                session_key,
                base_url,
                ..
            } => {
                let sk = session_key
                    .as_ref()
                    .ok_or_else(|| Error::Auth("Session key required".to_string()))?;

                let mut params = BTreeMap::new();
                params.insert("method".to_string(), "track.updateNowPlaying".to_string());
                params.insert("api_key".to_string(), api_key.clone());
                params.insert("sk".to_string(), sk.clone());
                params.insert("artist".to_string(), now_playing.artist.clone());
                params.insert("track".to_string(), now_playing.track.clone());

                if let Some(album) = &now_playing.album {
                    params.insert("album".to_string(), album.clone());
                }
                if let Some(track_number) = now_playing.track_number {
                    params.insert("trackNumber".to_string(), track_number.to_string());
                }
                if let Some(duration) = now_playing.duration {
                    params.insert("duration".to_string(), duration.to_string());
                }
                if let Some(album_artist) = &now_playing.album_artist {
                    params.insert("albumArtist".to_string(), album_artist.clone());
                }

                let sig = signature::generate(&params, api_secret);
                params.insert("api_sig".to_string(), sig);
                params.insert("format".to_string(), "json".to_string());

                let resp = self
                    .http_client
                    .post(base_url.clone())
                    .form(&params)
                    .send()
                    .await?
                    .error_for_status()?;

                let json: serde_json::Value = resp.json().await?;

                if json.get("error").is_some() {
                    Err(Error::Api(json["message"].as_str().unwrap().to_string()))
                } else {
                    Ok(())
                }
            }
            AuthMode::Token { base_url, token } => {
                let url = base_url.join("now")?;

                self.http_client
                    .post(url)
                    .bearer_auth(token)
                    .json(now_playing)
                    .send()
                    .await?
                    .error_for_status()?;

                Ok(())
            }
        }
    }

    /// Submit scrobble(s)
    pub async fn scrobble(&self, scrobbles: &[Scrobble]) -> Result<ScrobbleResponse> {
        if scrobbles.is_empty() {
            return Err(Error::InvalidParameter("No scrobbles provided".to_string()));
        }
        if scrobbles.len() > 50 {
            return Err(Error::InvalidParameter(
                "Maximum 50 scrobbles per request".to_string(),
            ));
        }

        match &self.auth {
            AuthMode::LastFm {
                api_key,
                api_secret,
                session_key,
                base_url,
                ..
            } => {
                let sk = session_key
                    .as_ref()
                    .ok_or_else(|| Error::Auth("Session key required".to_string()))?;

                let mut params = BTreeMap::new();
                params.insert("method".to_string(), "track.scrobble".to_string());
                params.insert("api_key".to_string(), api_key.clone());
                params.insert("sk".to_string(), sk.clone());

                for (i, scrobble) in scrobbles.iter().enumerate() {
                    params.insert(format!("artist[{}]", i), scrobble.artist.clone());
                    params.insert(format!("track[{}]", i), scrobble.track.clone());
                    params.insert(format!("timestamp[{}]", i), scrobble.timestamp.to_string());

                    if let Some(album) = &scrobble.album {
                        params.insert(format!("album[{}]", i), album.clone());
                    }
                    if let Some(track_number) = scrobble.track_number {
                        params.insert(format!("trackNumber[{}]", i), track_number.to_string());
                    }
                    if let Some(duration) = scrobble.duration {
                        params.insert(format!("duration[{}]", i), duration.to_string());
                    }
                    if let Some(album_artist) = &scrobble.album_artist {
                        params.insert(format!("albumArtist[{}]", i), album_artist.clone());
                    }
                }

                let sig = signature::generate(&params, api_secret);
                params.insert("api_sig".to_string(), sig);
                params.insert("format".to_string(), "json".to_string());

                let resp = self
                    .http_client
                    .post(base_url.clone())
                    .form(&params)
                    .send()
                    .await?
                    .error_for_status()?;

                let json: serde_json::Value = resp.json().await?;

                if let Some(error) = json.get("error") {
                    Err(Error::Api(error.to_string()))
                } else {
                    Ok(serde_json::from_value(json)?)
                }
            }
            AuthMode::Token { base_url, token } => {
                let url = base_url.join("scrob")?;

                self.http_client
                    .post(url)
                    .bearer_auth(token)
                    .json(&scrobbles)
                    .send()
                    .await?
                    .error_for_status()?;

                // Token mode: return a synthetic success response
                Ok(ScrobbleResponse {
                    scrobbles: crate::scrobble::ScrobbleData {
                        attr: crate::scrobble::ScrobbleAttr {
                            accepted: scrobbles.len() as u32,
                            ignored: 0,
                        },
                    },
                })
            }
        }
    }

    /// Get track information
    ///
    /// Retrieves metadata for a track including playcount, listeners, tags, and wiki content.
    /// This is a public endpoint that doesn't require authentication, but can optionally
    /// include user-specific data (playcount, loved status) if a username is provided.
    ///
    /// Only available in Last.fm mode.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// use last_fm_rs::Client;
    ///
    /// let client = Client::new("api_key", "secret");
    ///
    /// // Get public track info
    /// let track = client.track_get_info("Kendrick Lamar", "Wesley's Theory", None).await?;
    /// println!("Listeners: {}", track.listeners);
    /// println!("Playcount: {}", track.playcount);
    ///
    /// // Get track info with user-specific data
    /// let track = client.track_get_info("Kendrick Lamar", "Wesley's Theory", Some("username")).await?;
    /// if let Some(count) = track.userplaycount {
    ///   println!("You've played this {} times", count);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn track_get_info(
        &self,
        artist: &str,
        track: &str,
        username: Option<&str>,
    ) -> Result<crate::track::TrackInfo> {
        match &self.auth {
            AuthMode::LastFm {
                api_key, base_url, ..
            } => {
                let mut params = BTreeMap::new();
                params.insert("method".to_string(), "track.getInfo".to_string());
                params.insert("api_key".to_string(), api_key.clone());
                params.insert("artist".to_string(), artist.to_string());
                params.insert("track".to_string(), track.to_string());
                params.insert("format".to_string(), "json".to_string());

                if let Some(username) = username {
                    params.insert("username".to_string(), username.to_string());
                }

                let response = self
                    .http_client
                    .get(base_url.clone())
                    .query(&params)
                    .send()
                    .await?;

                let response_text = response.text().await?;
                let track_response: crate::track::TrackInfoResponse =
                    serde_json::from_str(&response_text).map_err(|e| Error::Json(e))?;

                Ok(track_response.track)
            }
            AuthMode::Token { .. } => Err(Error::InvalidParameter(
                "track.getInfo is only available in Last.fm mode".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = Client::new("test_key", "test_secret");
        assert!(client.auth.is_lastfm());
        assert_eq!(client.auth.api_key(), Some("test_key"));
        assert_eq!(client.auth.api_secret(), Some("test_secret"));
        assert!(client.auth.session_key().is_none());
    }

    #[test]
    fn test_client_with_session_key() {
        let client = Client::new("test_key", "test_secret").with_session_key("session123");
        assert_eq!(client.auth.session_key(), Some("session123"));
    }

    #[test]
    fn test_client_with_token() {
        let client =
            Client::with_token("https://scrob.example.com/api/", "my_token").expect("valid URL");
        assert!(client.auth.is_token());
    }

    #[test]
    fn test_client_with_token_invalid_url() {
        let result = Client::with_token("not a url", "token");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_auth_url() {
        let client = Client::new("my_api_key", "secret");
        let token = AuthToken {
            token: "test_token".to_string(),
        };
        let url = client.get_auth_url(&token).expect("valid auth URL");
        assert_eq!(
            url,
            "http://www.last.fm/api/auth/?api_key=my_api_key&token=test_token"
        );
    }

    #[test]
    fn test_with_api_base() {
        let client = Client::new("my_api_key", "secret")
            .with_api_base("https://scrob.example.com/2.0/")
            .expect("valid URL");
        // auth is Last.fm and the URL should be reflected in auth mode state
        assert!(client.auth.is_lastfm());
        // And get_auth_url should still default; only the base URL is overridden
        let token = AuthToken {
            token: "t".to_string(),
        };
        let url = client.get_auth_url(&token).expect("valid auth URL");
        assert!(url.starts_with("http://www.last.fm/api/auth/"));
    }

    #[test]
    fn test_with_api_base_invalid_url() {
        let result = Client::new("k", "s").with_api_base("not a url");
        assert!(result.is_err());
    }

    #[test]
    fn test_with_auth_url() {
        let client = Client::new("my_api_key", "secret")
            .with_auth_url("https://scrob.example.com/login/")
            .expect("valid URL");
        let token = AuthToken {
            token: "t".to_string(),
        };
        let url = client.get_auth_url(&token).expect("valid auth URL");
        assert_eq!(
            url,
            "https://scrob.example.com/login/?api_key=my_api_key&token=t"
        );
    }

    #[test]
    fn test_with_auth_url_invalid_url() {
        let result = Client::new("k", "s").with_auth_url("not a url");
        assert!(result.is_err());
    }

    #[test]
    fn test_url_builders_are_no_op_in_token_mode() {
        let client = Client::with_token("https://scrob.example.com/api/", "tok")
            .expect("valid URL")
            .with_api_base("https://other.example.com/api/")
            .expect("valid URL")
            .with_auth_url("https://other.example.com/auth/")
            .expect("valid URL");
        assert!(client.auth.is_token());
        let token = AuthToken {
            token: "t".to_string(),
        };
        // Auth URL builder should not have applied; still errors in token mode
        assert!(client.get_auth_url(&token).is_err());
    }

    #[test]
    fn test_get_auth_url_url_encodes_values() {
        let client = Client::new("key with space", "secret")
            .with_auth_url("https://scrob.example.com/auth/")
            .expect("valid URL");
        let token = AuthToken {
            token: "tok&en".to_string(),
        };
        let url = client.get_auth_url(&token).expect("valid auth URL");
        // url crate form-encodes spaces as '+' and percent-encodes other specials
        assert!(url.contains("key+with+space"));
        assert!(url.contains("tok%26en"));
    }

    #[test]
    fn test_get_auth_url_fails_in_token_mode() {
        let client =
            Client::with_token("https://scrob.example.com/api/", "token").expect("valid URL");
        let token = AuthToken {
            token: "test_token".to_string(),
        };
        let result = client.get_auth_url(&token);
        assert!(result.is_err());
    }

    #[test]
    fn test_url_join_behavior() {
        use url::Url;

        // Test with trailing slash
        let base_with_slash = Url::parse("http://localhost:3000/").unwrap();
        assert_eq!(
            base_with_slash.join("now").unwrap().as_str(),
            "http://localhost:3000/now"
        );
        assert_eq!(
            base_with_slash.join("scrob").unwrap().as_str(),
            "http://localhost:3000/scrob"
        );

        // Test without trailing slash
        let base_no_slash = Url::parse("http://localhost:3000").unwrap();
        assert_eq!(
            base_no_slash.join("now").unwrap().as_str(),
            "http://localhost:3000/now"
        );
        assert_eq!(
            base_no_slash.join("scrob").unwrap().as_str(),
            "http://localhost:3000/scrob"
        );
    }
}
