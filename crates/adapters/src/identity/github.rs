mod repository;
pub use repository::GitHubRepositoryProvider;

use async_trait::async_trait;
use factory_core::identity::{
    IdentityError, IdentityProvider, RepositoryAccessProvider, RepositoryAccessToken,
    VerifiedIdentity,
};
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet,
    EndpointSet, HttpRequest, HttpResponse, PkceCodeVerifier, RedirectUrl, RequestTokenError,
    Scope, TokenResponse, TokenUrl,
    basic::{BasicClient, BasicTokenType},
};
use reqwest::{Client, Response, StatusCode, header};
use serde::Deserialize;
use std::{fmt, time::Duration};
use url::{Host, Url};

const AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const PROFILE_URL: &str = "https://api.github.com/user";
const TOKEN_BODY_LIMIT: usize = 32 * 1024;
const PROFILE_BODY_LIMIT: usize = 64 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Public account lookup for an operator-selected administrator. This does not
/// authenticate the account; browser access still requires the normal OAuth flow.
pub async fn lookup_github_account(username: &str) -> Result<VerifiedIdentity, IdentityError> {
    if username.is_empty()
        || username.len() > 39
        || !username
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        || username.starts_with('-')
        || username.ends_with('-')
        || username.contains("--")
    {
        return Err(IdentityError::InvalidInput);
    }
    let url = Url::parse(&format!("https://api.github.com/users/{username}"))
        .map_err(|_| IdentityError::InvalidInput)?;
    lookup_account_at(&http_client(REQUEST_TIMEOUT)?, url).await
}

async fn lookup_account_at(http: &Client, url: Url) -> Result<VerifiedIdentity, IdentityError> {
    let response = http
        .get(url)
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|_| IdentityError::Unavailable)?;
    if response.status() == StatusCode::NOT_FOUND {
        return Err(IdentityError::NotFound);
    }
    if response.status() != StatusCode::OK {
        return Err(IdentityError::Unavailable);
    }
    let body = bounded_body(response, PROFILE_BODY_LIMIT)
        .await
        .map_err(|_| IdentityError::Unavailable)?;
    #[derive(Deserialize)]
    struct Account {
        #[serde(flatten)]
        profile: Profile,
        #[serde(rename = "type")]
        kind: String,
    }
    let account: Account =
        serde_json::from_slice(&body).map_err(|_| IdentityError::InvalidInput)?;
    if account.kind != "User" {
        return Err(IdentityError::InvalidInput);
    }
    account.profile.into_identity()
}

type OAuthClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

/// No Debug implementation: the OAuth client owns a client secret.
#[derive(Clone)]
pub struct GitHubProvider {
    oauth: OAuthClient,
    http: Client,
    profile: Url,
}

impl GitHubProvider {
    pub fn new(
        client_id: String,
        client_secret: String,
        callback: String,
    ) -> Result<Self, IdentityError> {
        if !valid_credential(&client_id, 256) || !valid_credential(&client_secret, 2048) {
            return Err(IdentityError::Unavailable);
        }
        validate_callback(&callback)?;
        let oauth = BasicClient::new(ClientId::new(client_id))
            .set_client_secret(ClientSecret::new(client_secret))
            .set_auth_type(AuthType::RequestBody)
            .set_auth_uri(
                AuthUrl::new(AUTHORIZE_URL.into()).map_err(|_| IdentityError::Unavailable)?,
            )
            .set_token_uri(TokenUrl::new(TOKEN_URL.into()).map_err(|_| IdentityError::Unavailable)?)
            .set_redirect_uri(RedirectUrl::new(callback).map_err(|_| IdentityError::Unavailable)?);
        Ok(Self {
            oauth,
            http: http_client(REQUEST_TIMEOUT)?,
            profile: Url::parse(PROFILE_URL).map_err(|_| IdentityError::Unavailable)?,
        })
    }

    async fn token_request(
        http: Client,
        request: HttpRequest,
    ) -> Result<HttpResponse, TransportFailure> {
        let (parts, body) = request.into_parts();
        let response = http
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .header(header::ACCEPT, "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| TransportFailure)?;
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = bounded_body(response, TOKEN_BODY_LIMIT).await?;
        // GitHub may return an OAuth error JSON object with status 200. Normalize
        // that documented provider shape so oauth2 parses its standard error type.
        let is_oauth_error = status == StatusCode::OK
            && serde_json::from_slice::<serde_json::Value>(&bytes)
                .is_ok_and(|json| json.get("error").is_some());
        let mut result = HttpResponse::new(bytes);
        *result.status_mut() = if is_oauth_error {
            StatusCode::BAD_REQUEST
        } else {
            status
        };
        *result.headers_mut() = headers;
        Ok(result)
    }
}

#[async_trait]
impl IdentityProvider for GitHubProvider {
    fn provider(&self) -> &str {
        "github"
    }
    fn issuer(&self) -> &str {
        "https://github.com"
    }

    fn authorization_url(&self, state: &str, challenge: &str) -> Result<String, IdentityError> {
        if !(32..=256).contains(&state.len())
            || !base64_url(state)
            || challenge.len() != 43
            || !base64_url(challenge)
        {
            return Err(IdentityError::InvalidInput);
        }
        // The HTTP host generates this challenge with oauth2's SHA-256 PKCE
        // primitive. The core port carries strings, keeping OAuth types outside core.
        let (url, _) = self
            .oauth
            .authorize_url(|| CsrfToken::new(state.to_owned()))
            .add_scope(Scope::new("read:user".into()))
            .add_scope(Scope::new("repo".into()))
            .add_scope(Scope::new("write:public_key".into()))
            .add_scope(Scope::new("user:email".into()))
            .add_scope(Scope::new("offline_access".into()))
            .add_extra_param("code_challenge", challenge)
            .add_extra_param("code_challenge_method", "S256")
            .url();
        Ok(url.into())
    }

    async fn exchange_login(
        &self,
        code: &str,
        verifier: &str,
    ) -> Result<(VerifiedIdentity, Option<RepositoryAccessToken>), IdentityError> {
        let provider = GitHubRepositoryProvider::from_identity(self.clone());
        let (token, identity) = RepositoryAccessProvider::exchange(&provider, code, verifier)
            .await
            .map_err(|error| match error {
                factory_core::identity::RepositoryAccessError::InvalidLogin => {
                    IdentityError::InvalidLogin
                }
                _ => IdentityError::Unavailable,
            })?;
        Ok((identity, Some(token)))
    }

    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
    ) -> Result<VerifiedIdentity, IdentityError> {
        if !valid_credential(code, 4096)
            || !(43..=128).contains(&verifier.len())
            || !verifier
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c))
        {
            return Err(IdentityError::InvalidLogin);
        }
        let http = self.http.clone();
        let transport = move |request| Self::token_request(http.clone(), request);
        let token = self
            .oauth
            .exchange_code(AuthorizationCode::new(code.to_owned()))
            .set_pkce_verifier(PkceCodeVerifier::new(verifier.to_owned()))
            .request_async(&transport)
            .await
            .map_err(|error| match error {
                RequestTokenError::ServerResponse(response)
                    if matches!(
                        response.error().as_ref(),
                        "invalid_grant" | "bad_verification_code" | "access_denied"
                    ) =>
                {
                    IdentityError::InvalidLogin
                }
                _ => IdentityError::Unavailable,
            })?;
        if token.token_type() != &BasicTokenType::Bearer
            || !valid_credential(token.access_token().secret(), 4096)
        {
            return Err(IdentityError::InvalidLogin);
        }
        let response = self
            .http
            .get(self.profile.clone())
            .bearer_auth(token.access_token().secret())
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|_| IdentityError::Unavailable)?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(IdentityError::InvalidLogin);
        }
        if response.status() != StatusCode::OK {
            return Err(IdentityError::Unavailable);
        }
        let body = bounded_body(response, PROFILE_BODY_LIMIT)
            .await
            .map_err(|_| IdentityError::Unavailable)?;
        let profile: Profile =
            serde_json::from_slice(&body).map_err(|_| IdentityError::InvalidLogin)?;
        // The token is dropped here; only verified identity metadata leaves this adapter.
        profile.into_identity()
    }
}

impl Profile {
    fn into_identity(self) -> Result<VerifiedIdentity, IdentityError> {
        if self.id == 0 {
            return Err(IdentityError::InvalidLogin);
        }
        let identity = VerifiedIdentity {
            provider: "github".into(),
            issuer: "https://github.com".into(),
            subject: self.id.to_string(),
            display_name: self
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| self.login.clone()),
            handle: self.login,
        };
        identity.validate()?;
        Ok(identity)
    }
}

#[derive(Deserialize)]
struct Profile {
    id: u64,
    login: String,
    name: Option<String>,
}

fn http_client(timeout: Duration) -> Result<Client, IdentityError> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .no_proxy()
        .referer(false)
        .connect_timeout(Duration::from_secs(3))
        .timeout(timeout)
        .user_agent("Factory-Identity/0.0.0")
        .build()
        .map_err(|_| IdentityError::Unavailable)
}

async fn bounded_body(mut response: Response, limit: usize) -> Result<Vec<u8>, TransportFailure> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(TransportFailure);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| TransportFailure)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(TransportFailure);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn base64_url(value: &str) -> bool {
    value
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
}

fn valid_credential(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn validate_callback(value: &str) -> Result<(), IdentityError> {
    let url = Url::parse(value).map_err(|_| IdentityError::Unavailable)?;
    let local = match url.host() {
        Some(Host::Domain("localhost")) => true,
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    let secure = url.scheme() == "https" && url.host().is_some();
    let loopback_http = url.scheme() == "http" && local;
    if !(secure || loopback_http)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/api/auth/github/callback"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() == Some(0)
    {
        return Err(IdentityError::Unavailable);
    }
    Ok(())
}

#[derive(Debug)]
struct TransportFailure;
impl fmt::Display for TransportFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Provider transport unavailable")
    }
}
impl std::error::Error for TransportFailure {}

#[cfg(test)]
mod tests;
