use super::AuthError;
use axum::http::{HeaderMap, HeaderValue, header};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use cookie::{Cookie, SameSite};
use factory_core::identity::{CredentialHash, IdentityError};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub(super) const SESSION_COOKIE: &str = "factory_session";
pub(super) const LOGIN_COOKIE: &str = "factory_login";

pub(super) fn random_token() -> Result<String, IdentityError> {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| IdentityError::Unavailable)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub(super) fn hash_token(value: &str) -> Result<CredentialHash, IdentityError> {
    if value.len() != 43 {
        return Err(IdentityError::InvalidLogin);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| IdentityError::InvalidLogin)?;
    if bytes.len() != 32 || URL_SAFE_NO_PAD.encode(bytes) != value {
        return Err(IdentityError::InvalidLogin);
    }
    Ok(CredentialHash(Sha256::digest(value.as_bytes()).into()))
}

pub(super) fn credential(
    headers: &HeaderMap,
    name: &str,
) -> Result<Option<CredentialHash>, AuthError> {
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        let text = value.to_str().map_err(|_| AuthError::invalid())?;
        if text.len() > 8192 {
            return Err(AuthError::invalid());
        }
        for cookie in Cookie::split_parse(text) {
            let cookie = cookie.map_err(|_| AuthError::invalid())?;
            if cookie.name() == name {
                if found.is_some() {
                    return Err(AuthError::invalid());
                }
                found = Some(hash_token(cookie.value()).map_err(|_| AuthError::invalid())?);
            }
        }
    }
    Ok(found)
}

pub(super) fn cookie(
    name: &'static str,
    value: String,
    lifetime: i64,
    secure: bool,
) -> HeaderValue {
    let value = Cookie::build((name, value))
        .http_only(true)
        .same_site(SameSite::Lax)
        .path("/")
        .secure(secure)
        .max_age(cookie::time::Duration::seconds(lifetime))
        .build()
        .to_string();
    HeaderValue::from_str(&value)
        .expect("Generated cookie contains only a fixed name and base64url token")
}

pub(super) fn clear_cookie(name: &'static str, secure: bool) -> HeaderValue {
    cookie(name, String::new(), 0, secure)
}
