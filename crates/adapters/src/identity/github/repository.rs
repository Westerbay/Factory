use super::*;
use factory_core::identity::{
    AccessibleRepository, AccessibleRepositoryPage, RepositoryAccessError as Error,
    RepositoryAccessProvider, RepositoryAccessToken, RepositoryDraft, RepositoryDraftDiff,
    RepositoryDraftDiffTarget, RepositoryDraftTarget, RepositoryOperationGuard,
    RepositoryOriginVerification,
};
use serde::Serialize;
use uuid::Uuid;

const PAGE_SIZE: usize = 100;
const MAX_PAGE: u32 = 1000;
const BODY_LIMIT: usize = 1024 * 1024;
const ERROR_BODY_LIMIT: usize = 16 * 1024;
const DRAFT_DIFF_FILES_LIMIT: usize = 3000;
const DRAFT_DIFF_PATHS_LIMIT: usize = 256 * 1024;

/// Repository operations and scoped OAuth token exchange.
pub struct GitHubRepositoryProvider {
    inner: GitHubProvider,
}

impl GitHubRepositoryProvider {
    pub(super) fn from_identity(inner: GitHubProvider) -> Self {
        Self { inner }
    }
    pub fn new(client_id: String, client_secret: String, callback: String) -> Result<Self, Error> {
        Ok(Self {
            inner: GitHubProvider::new(client_id, client_secret, callback)
                .map_err(|_| Error::Unavailable)?,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url, Error> {
        self.inner
            .profile
            .join(path)
            .map_err(|_| Error::Unavailable)
    }

    async fn get(
        &self,
        token: &RepositoryAccessToken,
        path: &str,
        page: u32,
        guard: Option<&dyn RepositoryOperationGuard>,
    ) -> Result<Vec<u8>, Error> {
        if page == 0 || page > MAX_PAGE {
            return Err(Error::InvalidInput);
        }
        if let Some(guard) = guard {
            guard.check().await?;
        }
        let mut url = self.endpoint(path)?;
        url.query_pairs_mut()
            .append_pair("per_page", "100")
            .append_pair("page", &page.to_string());
        let response = self
            .inner
            .http
            .get(url)
            .bearer_auth(token.secret())
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        if response.status() != StatusCode::OK {
            return Err(status_error(response).await);
        }
        bounded_body(response, BODY_LIMIT)
            .await
            .map_err(|_| Error::Unavailable)
    }

    async fn key_exists(
        &self,
        token: &RepositoryAccessToken,
        public_key: &str,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<bool, Error> {
        let expected = key_material(public_key).ok_or(Error::InvalidInput)?;
        // Fail closed if the list is too large rather than concluding absence.
        for page in 1..=10 {
            let body = self.get(token, "/user/keys", page, Some(guard)).await?;
            let keys: Vec<PublicKey> =
                serde_json::from_slice(&body).map_err(|_| Error::Unavailable)?;
            if keys.len() > PAGE_SIZE {
                return Err(Error::Unavailable);
            }
            if keys
                .iter()
                .any(|key| key_material(&key.key) == Some(expected))
            {
                return Ok(true);
            }
            if keys.len() < PAGE_SIZE {
                return Ok(false);
            }
        }
        Err(Error::Unavailable)
    }

    async fn pull_bytes(
        &self,
        token: &RepositoryAccessToken,
        path: &str,
        accept: &'static str,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<Vec<u8>, Error> {
        guard.check().await?;
        let response = self
            .inner
            .http
            .get(self.endpoint(path)?)
            .bearer_auth(token.secret())
            .header(header::ACCEPT, accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        if response.status() != StatusCode::OK {
            return Err(status_error(response).await);
        }
        bounded_body(response, BODY_LIMIT)
            .await
            .map_err(|_| Error::Unavailable)
    }
}

#[derive(Deserialize)]
struct PublicKey {
    key: String,
}
#[derive(Deserialize)]
struct Email {
    email: String,
    primary: bool,
    verified: bool,
}
#[derive(Deserialize)]
struct Repo {
    id: u64,
    full_name: String,
    private: bool,
}
#[derive(Serialize)]
struct NewKey<'a> {
    title: &'static str,
    key: &'a str,
}

#[derive(Serialize)]
struct NewDraft<'a> {
    title: &'a str,
    head: &'a str,
    base: &'a str,
    body: &'a str,
    draft: bool,
}

#[derive(Deserialize, PartialEq, Eq)]
struct PullRequest {
    number: u64,
    state: String,
    draft: bool,
    body: Option<String>,
    head: PullBranch,
    base: PullBranch,
    changed_files: Option<usize>,
    updated_at: Option<String>,
}

impl PullRequest {
    fn has_operation(&self, operation_id: Uuid) -> bool {
        let marker = operation_marker(operation_id);
        self.body
            .as_deref()
            .is_some_and(|body| body.lines().any(|line| line == marker))
    }
}

#[derive(Deserialize, PartialEq, Eq)]
struct PullBranch {
    #[serde(rename = "ref")]
    reference: String,
    sha: String,
    repo: Option<PullRepository>,
}

impl PullBranch {
    fn matches_repository(&self, id: i64, full_name: &str) -> bool {
        self.repo
            .as_ref()
            .is_some_and(|repo| repo.id == id && repo.full_name == full_name)
    }
}

#[derive(Deserialize, PartialEq, Eq)]
struct PullRepository {
    id: i64,
    full_name: String,
}

#[derive(Deserialize)]
struct PullFile {
    filename: String,
}

fn operation_marker(operation_id: Uuid) -> String {
    format!("<!-- factory-feature-delivery:{operation_id} -->")
}

fn valid_commit_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64)
        && oid
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_base_branch(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && !value.starts_with('-')
        && !value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == ':')
}

fn validate_draft_target(target: &RepositoryDraftTarget<'_>) -> Result<(), Error> {
    let valid_head = target
        .head
        .strip_prefix("factory/features/")
        .and_then(|value| value.parse::<Uuid>().ok())
        .is_some_and(|id| !id.is_nil());
    if target.repository_id <= 0
        || !valid_repo_name(target.full_name)
        || !valid_head
        || target.operation_id.is_nil()
        || !valid_commit_oid(target.head_oid)
        || !valid_base_branch(target.base)
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

fn verified_draft(
    pull: PullRequest,
    target: &RepositoryDraftTarget<'_>,
) -> Result<RepositoryDraft, Error> {
    if pull.number == 0
        || !pull.draft
        || pull.state != "open"
        || pull.head.reference != target.head
        || pull.base.reference != target.base
        || !pull
            .head
            .matches_repository(target.repository_id, target.full_name)
        || !pull
            .base
            .matches_repository(target.repository_id, target.full_name)
        || !valid_commit_oid(&pull.head.sha)
        || !valid_commit_oid(&pull.base.sha)
        || !pull.has_operation(target.operation_id)
    {
        return Err(Error::Conflict);
    }
    Ok(RepositoryDraft {
        url: format!(
            "https://github.com/{}/pull/{}",
            target.full_name, pull.number
        ),
        base_oid: pull.base.sha,
        head_oid: pull.head.sha,
    })
}

fn draft_diff_number(target: &RepositoryDraftDiffTarget<'_>) -> Result<u64, Error> {
    if target.repository_id <= 0
        || !valid_repo_name(target.full_name)
        || target.feature_id.is_nil()
        || target.operation_id.is_nil()
    {
        return Err(Error::InvalidInput);
    }
    let prefix = format!("https://github.com/{}/pull/", target.full_name);
    let tail = target
        .draft_url
        .strip_prefix(&prefix)
        .ok_or(Error::Conflict)?;
    let number: u64 = tail.parse().map_err(|_| Error::Conflict)?;
    if number == 0 || number.to_string() != tail {
        return Err(Error::Conflict);
    }
    Ok(number)
}

fn verify_diff_metadata(
    pull: &PullRequest,
    target: &RepositoryDraftDiffTarget<'_>,
    number: u64,
) -> Result<usize, Error> {
    if pull.number != number
        || pull.head.reference != format!("factory/features/{}", target.feature_id)
        || !pull
            .head
            .matches_repository(target.repository_id, target.full_name)
        || !pull
            .base
            .matches_repository(target.repository_id, target.full_name)
        || !valid_base_branch(&pull.base.reference)
        || !valid_commit_oid(&pull.head.sha)
        || !valid_commit_oid(&pull.base.sha)
        || !pull.has_operation(target.operation_id)
    {
        return Err(Error::Conflict);
    }
    pull.changed_files
        .filter(|count| *count <= DRAFT_DIFF_FILES_LIMIT)
        .filter(|_| {
            pull.updated_at
                .as_ref()
                .is_some_and(|timestamp| !timestamp.is_empty() && timestamp.len() <= 64)
        })
        .ok_or(Error::Unavailable)
}

fn check_status(status: StatusCode, headers: &header::HeaderMap) -> Result<(), Error> {
    match status {
        StatusCode::OK => Ok(()),
        StatusCode::UNAUTHORIZED => Err(Error::ProviderUnauthorized),
        StatusCode::FORBIDDEN
            if headers
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                == Some("0")
                || headers
                    .get(header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .is_some() =>
        {
            Err(Error::RateLimited)
        }
        StatusCode::FORBIDDEN => Err(Error::ProviderForbidden),
        StatusCode::TOO_MANY_REQUESTS => Err(Error::RateLimited),
        _ => Err(Error::Unavailable),
    }
}

/// Parse only a bounded structured error; never expose provider body text.
async fn status_error(response: Response) -> Error {
    let status = response.status();
    let error = match check_status(status, response.headers()) {
        Ok(()) => return Error::Unavailable,
        Err(error) => error,
    };
    if status != StatusCode::FORBIDDEN || error != Error::ProviderForbidden {
        return error;
    }
    #[derive(Deserialize)]
    struct ErrorBody {
        message: Option<String>,
    }
    if let Ok(body) = bounded_body(response, ERROR_BODY_LIMIT).await
        && let Ok(parsed) = serde_json::from_slice::<ErrorBody>(&body)
        && parsed.message.as_deref().is_some_and(|message| {
            message
                .to_ascii_lowercase()
                .starts_with("you have exceeded a secondary rate limit")
        })
    {
        return Error::RateLimited;
    }
    Error::ProviderForbidden
}

fn key_material(key: &str) -> Option<(&str, &str)> {
    if key.len() > 1024 || key.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
        return None;
    }
    let mut parts = key.split_whitespace();
    let kind = parts.next()?;
    let value = parts.next()?;
    if kind != "ssh-ed25519"
        || value.len() < 40
        || value.len() > 128
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'='))
    {
        return None;
    }
    Some((kind, value))
}

fn valid_repo_name(value: &str) -> bool {
    if value.len() > 256 {
        return false;
    }
    let Some((owner, name)) = value.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && !name.is_empty()
        && owner.len() <= 100
        && name.len() <= 150
        && owner
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        && name != "."
        && name != ".."
}

#[async_trait]
impl RepositoryAccessProvider for GitHubRepositoryProvider {
    fn authorization_url(&self, state: &str, challenge: &str) -> Result<String, Error> {
        if !state
            .strip_prefix("repo.")
            .is_some_and(|token| token.len() == 43 && base64_url(token))
            || challenge.len() != 43
            || !base64_url(challenge)
        {
            return Err(Error::InvalidInput);
        }
        let (url, _) = self
            .inner
            .oauth
            .authorize_url(|| CsrfToken::new(state.to_owned()))
            .add_scope(Scope::new("repo".into()))
            .add_scope(Scope::new("write:public_key".into()))
            .add_scope(Scope::new("user:email".into()))
            .add_extra_param("code_challenge", challenge)
            .add_extra_param("code_challenge_method", "S256")
            .url();
        Ok(url.into())
    }

    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
    ) -> Result<(RepositoryAccessToken, VerifiedIdentity), Error> {
        if !valid_credential(code, 4096)
            || !(43..=128).contains(&verifier.len())
            || !verifier
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-._~".contains(&c))
        {
            return Err(Error::InvalidLogin);
        }
        let http = self.inner.http.clone();
        let transport = move |request| GitHubProvider::token_request(http.clone(), request);
        let token = self
            .inner
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
                    Error::InvalidLogin
                }
                _ => Error::Unavailable,
            })?;
        if token.token_type() != &BasicTokenType::Bearer {
            return Err(Error::InvalidLogin);
        }
        let granted = token.scopes().ok_or(Error::Forbidden)?;
        let values: Vec<&str> = granted
            .iter()
            .flat_map(|scope| scope.as_ref().split([',', ' ']))
            .collect();
        if !values.contains(&"repo")
            || !(values.contains(&"write:public_key") || values.contains(&"admin:public_key"))
            || !(values.contains(&"user:email") || values.contains(&"user"))
        {
            return Err(Error::Forbidden);
        }
        let token = RepositoryAccessToken::new(token.access_token().secret().to_owned())?;
        let response = self
            .inner
            .http
            .get(self.inner.profile.clone())
            .bearer_auth(token.secret())
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        if response.status() != StatusCode::OK {
            return Err(status_error(response).await);
        }
        let body = bounded_body(response, PROFILE_BODY_LIMIT)
            .await
            .map_err(|_| Error::Unavailable)?;
        let profile: Profile = serde_json::from_slice(&body).map_err(|_| Error::InvalidLogin)?;
        let identity = profile.into_identity().map_err(|_| Error::InvalidLogin)?;
        Ok((token, identity))
    }

    async fn primary_email(
        &self,
        token: &RepositoryAccessToken,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<String, Error> {
        for page in 1..=10 {
            let body = self.get(token, "/user/emails", page, Some(guard)).await?;
            let emails: Vec<Email> =
                serde_json::from_slice(&body).map_err(|_| Error::Unavailable)?;
            if emails.len() > PAGE_SIZE {
                return Err(Error::Unavailable);
            }
            if let Some(email) = emails.iter().find(|e| e.primary && e.verified) {
                if email.email.len() < 3
                    || email.email.len() > 320
                    || email.email.chars().any(char::is_control)
                {
                    return Err(Error::EmailUnavailable);
                }
                return Ok(email.email.clone());
            }
            if emails.len() < PAGE_SIZE {
                return Err(Error::EmailUnavailable);
            }
        }
        Err(Error::Unavailable)
    }

    async fn repositories(
        &self,
        token: &RepositoryAccessToken,
        page: u32,
    ) -> Result<AccessibleRepositoryPage, Error> {
        let body = self.get(token, "/user/repos", page, None).await?;
        let repos: Vec<Repo> = serde_json::from_slice(&body).map_err(|_| Error::Unavailable)?;
        if repos.len() > PAGE_SIZE {
            return Err(Error::Unavailable);
        }
        let next_page = if repos.len() == PAGE_SIZE && page < MAX_PAGE {
            Some(page + 1)
        } else {
            None
        };
        let mut items = Vec::with_capacity(repos.len());
        for repo in repos {
            let id = i64::try_from(repo.id).map_err(|_| Error::Unavailable)?;
            if id <= 0 || !valid_repo_name(&repo.full_name) {
                return Err(Error::Unavailable);
            }
            items.push(AccessibleRepository {
                id,
                full_name: repo.full_name,
                private: repo.private,
            });
        }
        Ok(AccessibleRepositoryPage { items, next_page })
    }

    async fn verify_clone_origin(
        &self,
        token: &RepositoryAccessToken,
        repository_id: i64,
        cloned_full_name: &str,
    ) -> Result<RepositoryOriginVerification, Error> {
        if repository_id <= 0 || !valid_repo_name(cloned_full_name) {
            return Err(Error::InvalidInput);
        }
        let path = format!("/repos/{cloned_full_name}");
        let response = self
            .inner
            .http
            .get(self.endpoint(&path)?)
            .bearer_auth(token.secret())
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        if response.status().is_redirection() || response.status() == StatusCode::NOT_FOUND {
            return Ok(RepositoryOriginVerification::Changed);
        }
        if response.status() != StatusCode::OK {
            return Err(status_error(response).await);
        }
        let body = bounded_body(response, BODY_LIMIT)
            .await
            .map_err(|_| Error::Unavailable)?;
        let repo: Repo = serde_json::from_slice(&body).map_err(|_| Error::Unavailable)?;
        let id = i64::try_from(repo.id).map_err(|_| Error::Unavailable)?;
        if id <= 0 || !valid_repo_name(&repo.full_name) {
            return Err(Error::Unavailable);
        }
        Ok(
            if id == repository_id && repo.full_name == cloned_full_name {
                RepositoryOriginVerification::Matched
            } else {
                RepositoryOriginVerification::Changed
            },
        )
    }

    async fn public_key_registered(
        &self,
        token: &RepositoryAccessToken,
        public_key: &str,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<bool, Error> {
        self.key_exists(token, public_key, guard).await
    }

    async fn ensure_public_key(
        &self,
        token: &RepositoryAccessToken,
        public_key: &str,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<(), Error> {
        if key_material(public_key).is_none() {
            return Err(Error::InvalidInput);
        }
        if self.key_exists(token, public_key, guard).await? {
            return Ok(());
        }
        guard.check().await?;
        let response = self
            .inner
            .http
            .post(self.endpoint("/user/keys")?)
            .bearer_auth(token.secret())
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&NewKey {
                title: "Factory environment",
                key: public_key,
            })
            .send()
            .await;
        // A POST can succeed even when its response is lost. Always reconcile by
        // exact public key and never issue a second POST automatically.
        if self.key_exists(token, public_key, guard).await? {
            return Ok(());
        }
        match response {
            Ok(response) => Err(status_error(response).await),
            Err(_) => Err(Error::Unavailable),
        }
    }

    async fn find_draft(
        &self,
        token: &RepositoryAccessToken,
        target: &RepositoryDraftTarget<'_>,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<Option<RepositoryDraft>, Error> {
        validate_draft_target(target)?;
        let (owner, _) = target
            .full_name
            .split_once('/')
            .ok_or(Error::InvalidInput)?;
        let head = format!("{owner}:{}", target.head);
        let path = format!("/repos/{}/pulls", target.full_name);
        guard.check().await?;
        let response = self
            .inner
            .http
            .get(self.endpoint(&path)?)
            .query(&[("head", head.as_str()), ("state", "all"), ("per_page", "2")])
            .bearer_auth(token.secret())
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        if response.status() != StatusCode::OK {
            return Err(status_error(response).await);
        }
        // Do not treat a partial listing as proof of absence or uniqueness.
        if response.headers().contains_key(header::LINK) {
            return Err(Error::Conflict);
        }
        let body = bounded_body(response, BODY_LIMIT)
            .await
            .map_err(|_| Error::Unavailable)?;
        let mut pulls: Vec<PullRequest> =
            serde_json::from_slice(&body).map_err(|_| Error::Unavailable)?;
        match pulls.len() {
            0 => Ok(None),
            1 => verified_draft(pulls.remove(0), target).map(Some),
            _ => Err(Error::Conflict),
        }
    }

    async fn create_draft(
        &self,
        token: &RepositoryAccessToken,
        target: &RepositoryDraftTarget<'_>,
        title: &str,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<RepositoryDraft, Error> {
        validate_draft_target(target)?;
        if title.trim().is_empty() || title.len() > 256 || title.chars().any(char::is_control) {
            return Err(Error::InvalidInput);
        }
        let path = format!("/repos/{}/pulls", target.full_name);
        let body = format!(
            "This draft contains the feature reviewed in Factory.\n\n{}",
            operation_marker(target.operation_id)
        );
        guard.check().await?;
        let response = self
            .inner
            .http
            .post(self.endpoint(&path)?)
            .bearer_auth(token.secret())
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&NewDraft {
                title,
                head: target.head,
                base: target.base,
                body: &body,
                draft: true,
            })
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        match response.status() {
            StatusCode::CREATED => {}
            StatusCode::CONFLICT | StatusCode::UNPROCESSABLE_ENTITY => return Err(Error::Conflict),
            StatusCode::NOT_FOUND => return Err(Error::ProviderForbidden),
            _ => return Err(status_error(response).await),
        }
        let body = bounded_body(response, BODY_LIMIT)
            .await
            .map_err(|_| Error::Unavailable)?;
        let pull = serde_json::from_slice(&body).map_err(|_| Error::Unavailable)?;
        // A rejected response after 201 may still correspond to an actual write.
        verified_draft(pull, target).map_err(|_| Error::Unavailable)
    }

    async fn draft_diff(
        &self,
        token: &RepositoryAccessToken,
        target: &RepositoryDraftDiffTarget<'_>,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<RepositoryDraftDiff, Error> {
        let number = draft_diff_number(target)?;
        let path = format!("/repos/{}/pulls/{number}", target.full_name);
        let metadata = || async {
            let bytes = self
                .pull_bytes(token, &path, "application/vnd.github+json", guard)
                .await?;
            serde_json::from_slice::<PullRequest>(&bytes).map_err(|_| Error::Unavailable)
        };
        let before = metadata().await?;
        let count = verify_diff_metadata(&before, target, number)?;
        let patch = self
            .pull_bytes(token, &path, "application/vnd.github.diff", guard)
            .await?;
        let patch = String::from_utf8(patch).map_err(|_| Error::Unavailable)?;
        if (count > 0 && !patch.starts_with("diff --git "))
            || (count == 0 && !patch.trim().is_empty())
        {
            return Err(Error::Unavailable);
        }

        let files_path = format!("{path}/files");
        let pages = u32::try_from(count.div_ceil(PAGE_SIZE)).map_err(|_| Error::Unavailable)?;
        let mut paths = Vec::with_capacity(count);
        let mut path_bytes = 0;
        for page in 1..=pages {
            let body = self.get(token, &files_path, page, Some(guard)).await?;
            let files: Vec<PullFile> =
                serde_json::from_slice(&body).map_err(|_| Error::Unavailable)?;
            if files.len() != PAGE_SIZE.min(count - paths.len()) {
                return Err(Error::Conflict);
            }
            for file in files {
                path_bytes += file.filename.len();
                if file.filename.is_empty()
                    || file.filename.contains('\0')
                    || path_bytes > DRAFT_DIFF_PATHS_LIMIT
                {
                    return Err(Error::Unavailable);
                }
                paths.push(file.filename);
            }
        }
        paths.sort_unstable();
        if paths.windows(2).any(|pair| pair[0] == pair[1]) || metadata().await? != before {
            return Err(Error::Conflict);
        }
        guard.check().await?;
        let patch_digest = ring::digest::digest(&ring::digest::SHA256, patch.as_bytes())
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(RepositoryDraftDiff {
            base_oid: before.base.sha,
            head_oid: before.head.sha,
            patch_digest,
            patch,
            changed_paths: paths,
        })
    }
}

#[cfg(test)]
mod tests;
