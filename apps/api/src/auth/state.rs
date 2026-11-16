use super::{
    AuthError,
    credentials::{SESSION_COOKIE, credential},
};
use crate::config::RuntimeConfig;
use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, header, request::Parts},
};
use factory_application::identity::IdentityService;
use factory_core::identity::{ActiveSession, CredentialHash, IdentityError};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

#[derive(Clone)]
pub struct AuthState {
    service: Arc<IdentityService>,
    pub(super) web_origin: String,
    shares_web_origin: bool,
    pub(super) secure_cookie: bool,
    quotas: Arc<Mutex<[Window; 2]>>,
    pub(crate) projects: Option<Arc<crate::projects::runtime::ProjectsRuntime>>,
}

pub struct Authenticated {
    pub token_hash: CredentialHash,
    pub session: ActiveSession,
}

/// Reject requests when identity is not configured, without constructing dummy services.
pub struct RequiredIdentity(pub AuthState);

impl FromRequestParts<Option<AuthState>> for RequiredIdentity {
    type Rejection = AuthError;

    async fn from_request_parts(
        _parts: &mut Parts,
        state: &Option<AuthState>,
    ) -> Result<Self, Self::Rejection> {
        state
            .clone()
            .map(Self)
            .ok_or_else(|| IdentityError::Unavailable.into())
    }
}

struct Window {
    start: Instant,
    count: u32,
}

impl AuthState {
    pub fn new(service: Arc<IdentityService>, config: &RuntimeConfig) -> Self {
        Self {
            service,
            web_origin: config.web_origin().to_owned(),
            shares_web_origin: config.api_origin() == config.web_origin(),
            secure_cookie: config.api_origin().starts_with("https://"),
            quotas: Arc::new(Mutex::new(std::array::from_fn(|_| Window {
                start: Instant::now(),
                count: 0,
            }))),
            projects: None,
        }
    }

    pub fn service(&self) -> &IdentityService {
        &self.service
    }

    pub(crate) fn with_projects(
        mut self,
        projects: Option<Arc<crate::projects::runtime::ProjectsRuntime>>,
    ) -> Self {
        self.projects = projects;
        self
    }
    pub(crate) fn web_origin(&self) -> &str {
        &self.web_origin
    }
    pub(crate) fn secure_cookie(&self) -> bool {
        self.secure_cookie
    }

    pub async fn authenticate(&self, headers: &HeaderMap) -> Result<Authenticated, AuthError> {
        let hash = credential(headers, SESSION_COOKIE)
            .map_err(|_| AuthError::from(IdentityError::Unauthorized))?
            .ok_or_else(|| AuthError::from(IdentityError::Unauthorized))?;
        let session = self.service.session(hash).await?;
        Ok(Authenticated {
            token_hash: hash,
            session,
        })
    }

    pub fn authorize_mutation(
        &self,
        headers: &HeaderMap,
        auth: &Authenticated,
        has_body: bool,
    ) -> Result<(), AuthError> {
        let origin =
            single_header(headers, header::ORIGIN.as_str()).ok_or_else(AuthError::forbidden)?;
        let csrf = single_header(headers, "x-csrf-token").ok_or_else(AuthError::forbidden)?;
        if origin != self.web_origin
            || csrf.len() != auth.session.csrf_token.len()
            || !bool::from(csrf.as_bytes().ct_eq(auth.session.csrf_token.as_bytes()))
        {
            return Err(AuthError::forbidden());
        }
        if has_body
            && !single_header(headers, header::CONTENT_TYPE.as_str()).is_some_and(|v| {
                v.split(';')
                    .next()
                    .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
            })
        {
            return Err(AuthError::invalid());
        }
        Ok(())
    }

    /// WebSocket upgrades do not inherit browser CORS enforcement.
    pub fn authorize_origin(&self, headers: &HeaderMap) -> Result<(), AuthError> {
        if single_header(headers, header::ORIGIN.as_str()) == Some(self.web_origin.as_str()) {
            Ok(())
        } else {
            Err(AuthError::forbidden())
        }
    }

    pub(crate) fn authorize_event_stream_origin(
        &self,
        headers: &HeaderMap,
    ) -> Result<(), AuthError> {
        // A same-origin GET does not normally carry Origin. Only use browser
        // metadata when Origin is absent, so invalid or duplicate values fail closed.
        if self.shares_web_origin
            && !headers.contains_key(header::ORIGIN)
            && single_header(headers, "sec-fetch-site") == Some("same-origin")
        {
            return Ok(());
        }
        self.authorize_origin(headers)
    }

    /// Fixed-size process-local quotas bound anonymous OAuth work without retaining IPs.
    /// They are deliberately shared by local users; distributed admission comes later.
    pub fn check_quota(&self, login: bool) -> Result<(), AuthError> {
        let mut quotas = self
            .quotas
            .lock()
            .map_err(|_| AuthError::from(IdentityError::Unavailable))?;
        let window = &mut quotas[usize::from(login)];
        if window.start.elapsed() >= Duration::from_secs(60) {
            window.start = Instant::now();
            window.count = 0;
        }
        let limit = if login { 60 } else { 600 };
        if window.count >= limit {
            return Err(AuthError::rate_limited());
        }
        window.count += 1;
        Ok(())
    }
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        None
    } else {
        Some(value)
    }
}
