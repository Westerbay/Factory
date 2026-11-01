mod contracts;
pub(crate) mod runtime;
use self::{contracts::*, runtime::ProjectsRuntime};
use crate::{
    auth::{AuthError, AuthState, cookie, hash_token, random_token},
    contracts::ApiError,
};
use axum::{
    Json,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use factory_core::{
    environments::{DesiredState, EnvironmentActor, ObservedState},
    identity::{
        IdentityError, RepositoryAccessError, RepositoryAuthorizationAttempt,
        RepositoryOriginVerification,
    },
    projects::ProjectError,
};
use std::{collections::HashSet, sync::Arc};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

pub(crate) const CONNECT_COOKIE: &str = "factory_github_connect";
const ATTEMPT_SECONDS: i64 = 600;
#[derive(Clone)]
struct StateData {
    auth: Option<AuthState>,
    runtime: Option<Arc<ProjectsRuntime>>,
}
impl StateData {
    async fn actor(
        &self,
        headers: &HeaderMap,
        mutation: Option<bool>,
    ) -> Result<(AuthState, EnvironmentActor), Response> {
        let auth = self
            .auth
            .clone()
            .ok_or_else(|| AuthError::from(IdentityError::Unavailable).into_response())?;
        auth.check_quota(false)
            .map_err(IntoResponse::into_response)?;
        let user = auth
            .authenticate(headers)
            .await
            .map_err(IntoResponse::into_response)?;
        if let Some(has_body) = mutation {
            auth.authorize_mutation(headers, &user, has_body)
                .map_err(IntoResponse::into_response)?;
        }
        Ok((
            auth,
            EnvironmentActor {
                user_id: user.session.user.id,
                session_hash: user.token_hash,
            },
        ))
    }
    fn runtime(&self) -> Result<Arc<ProjectsRuntime>, RepositoryAccessError> {
        self.runtime
            .clone()
            .ok_or(RepositoryAccessError::Unavailable)
    }
}
pub(crate) fn documented_router(
    auth: Option<AuthState>,
    runtime: Option<Arc<ProjectsRuntime>>,
) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list, import))
        .routes(routes!(recoverable))
        .routes(routes!(connection))
        .routes(routes!(setup))
        .routes(routes!(connect))
        .routes(routes!(disconnect))
        .routes(routes!(repositories))
        .routes(routes!(get, delete))
        .routes(routes!(verification))
        .routes(routes!(recover))
        .routes(routes!(retry))
        .layer(DefaultBodyLimit::max(8192))
        .with_state(StateData { auth, runtime })
}
fn response(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    (
        status,
        Json(ApiError {
            code: code.into(),
            message: message.into(),
            request_id: Uuid::new_v4(),
        }),
    )
        .into_response()
}
fn access_error(error: RepositoryAccessError) -> Response {
    use RepositoryAccessError::*;
    let (status, code, message) = match error {
        Unauthorized => (
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "An active Factory session is required.",
        ),
        ProviderUnauthorized => (
            StatusCode::CONFLICT,
            "provider_unauthorized",
            "Reconnect GitHub repository access.",
        ),
        ProviderForbidden => (
            StatusCode::CONFLICT,
            "provider_forbidden",
            "GitHub denied access. Check organization permissions.",
        ),
        RateLimited => (
            StatusCode::TOO_MANY_REQUESTS,
            "provider_rate_limited",
            "GitHub rate limit reached. Try again later.",
        ),
        EmailUnavailable => (
            StatusCode::CONFLICT,
            "email_unavailable",
            "GitHub did not return a verified primary email.",
        ),
        InvalidLogin => (
            StatusCode::CONFLICT,
            "invalid_github_authorization",
            "The GitHub authorization attempt is invalid or expired.",
        ),
        IdentityMismatch => (
            StatusCode::CONFLICT,
            "invalid_github_identity",
            "The authorized GitHub account did not match.",
        ),
        Forbidden => (
            StatusCode::CONFLICT,
            "forbidden",
            "Repository access denied.",
        ),
        InvalidInput => (
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "Invalid project request.",
        ),
        DestinationExists => (
            StatusCode::CONFLICT,
            "project_directory_exists",
            "A file or directory with this repository name already exists in Documents.",
        ),
        Conflict => (
            StatusCode::CONFLICT,
            "conflict",
            "The project or environment changed.",
        ),
        Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "Repository access is unavailable.",
        ),
    };
    response(status, code, message)
}
fn project_error(error: ProjectError) -> Response {
    match error {
        ProjectError::Unauthorized => access_error(RepositoryAccessError::Unauthorized),
        ProjectError::NotFound => {
            response(StatusCode::NOT_FOUND, "not_found", "Project not found.")
        }
        ProjectError::Conflict => access_error(RepositoryAccessError::Conflict),
        ProjectError::DestinationExists => access_error(RepositoryAccessError::DestinationExists),
        ProjectError::InUse => response(
            StatusCode::CONFLICT,
            "project_in_use",
            "Stop project runs and finish import recovery before deleting.",
        ),
        ProjectError::Invalid => access_error(RepositoryAccessError::InvalidInput),
        ProjectError::Unavailable => access_error(RepositoryAccessError::Unavailable),
    }
}
#[utoipa::path(get,path="/api/projects/connection",operation_id="getProjectConnection",security(("sessionCookie"=[])),responses((status=200,body=ConnectionView),(status=401,body=ApiError),(status=409,body=ApiError),(status=503,body=ApiError),(status=400,body=ApiError),(status=429,body=ApiError)))]
async fn connection(
    State(s): State<StateData>,
    h: HeaderMap,
) -> Result<Json<ConnectionView>, Response> {
    let (_, actor) = s.actor(&h, None).await?;
    let rt = s.runtime().map_err(access_error)?;
    let connection = rt.connection(actor).await.map_err(access_error)?;
    Ok(Json(connection.into()))
}
#[utoipa::path(post,path="/api/projects/setup",operation_id="setupProjectGithub",security(("sessionCookie"=[])),params(("Origin"=String,Header,description="Exact frontend Origin"),("X-CSRF-Token"=String,Header,description="Session CSRF token")),responses((status=204),(status=401,body=ApiError),(status=403,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn setup(
    State(s): State<StateData>,
    h: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, Response> {
    let (_, actor) = s.actor(&h, Some(false)).await?;
    if !body.is_empty() {
        return Err(access_error(RepositoryAccessError::InvalidInput));
    }
    let rt = s.runtime().map_err(access_error)?;
    // A disconnected browser must not abandon a submitted GitHub key request.
    tokio::spawn(async move {
        let (_, repository_actor) = rt.actor(actor).await?;
        rt.access()?.register_existing_key(repository_actor).await?;
        rt.ensure_identity(actor).await
    })
    .await
    .map_err(|_| access_error(RepositoryAccessError::Unavailable))?
    .map_err(access_error)?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(post,path="/api/projects/connect",operation_id="connectProjectGithub",security(("sessionCookie"=[])),params(("Origin"=String,Header,description="Exact frontend Origin"),("X-CSRF-Token"=String,Header,description="Session CSRF token")),responses((status=200,body=AuthorizationView),(status=401,body=ApiError),(status=403,body=ApiError),(status=503,body=ApiError),(status=400,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError)))]
async fn connect(
    State(s): State<StateData>,
    h: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, Response> {
    let (auth, actor) = s.actor(&h, Some(false)).await?;
    if !body.is_empty() {
        return Err(access_error(RepositoryAccessError::InvalidInput));
    };
    let rt = s.runtime().map_err(access_error)?;
    let (_, repo_actor) = rt.actor(actor).await.map_err(access_error)?;
    let state_token = random_token().map_err(|e| AuthError::from(e).into_response())?;
    let state = format!("repo.{state_token}");
    let binding = random_token().map_err(|e| AuthError::from(e).into_response())?;
    let verifier = oauth2::PkceCodeVerifier::new(
        random_token().map_err(|e| AuthError::from(e).into_response())?,
    );
    let challenge = oauth2::PkceCodeChallenge::from_code_verifier_sha256(&verifier);
    let attempt = RepositoryAuthorizationAttempt {
        state_hash: hash_token(&state_token).map_err(|e| AuthError::from(e).into_response())?,
        binding_hash: hash_token(&binding).map_err(|e| AuthError::from(e).into_response())?,
        verifier: verifier.secret().to_owned(),
        actor: repo_actor,
        authorization_epoch: 0,
    };
    let authorization_url = rt
        .access()
        .map_err(access_error)?
        .begin(attempt, &state, challenge.as_str())
        .await
        .map_err(access_error)?;
    let mut result = Json(AuthorizationView { authorization_url }).into_response();
    result.headers_mut().append(
        header::SET_COOKIE,
        cookie(
            CONNECT_COOKIE,
            binding,
            ATTEMPT_SECONDS,
            auth.secure_cookie(),
        ),
    );
    Ok(result)
}
#[utoipa::path(post,path="/api/projects/disconnect",operation_id="disconnectProjectGithub",security(("sessionCookie"=[])),params(("Origin"=String,Header,description="Exact frontend Origin"),("X-CSRF-Token"=String,Header,description="Session CSRF token")),responses((status=204),(status=401,body=ApiError),(status=403,body=ApiError),(status=503,body=ApiError),(status=400,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError)))]
async fn disconnect(
    State(s): State<StateData>,
    h: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, Response> {
    let (_, actor) = s.actor(&h, Some(false)).await?;
    if !body.is_empty() {
        return Err(access_error(RepositoryAccessError::InvalidInput));
    }
    let rt = s.runtime().map_err(access_error)?;
    let (_, repo_actor) = rt.actor(actor).await.map_err(access_error)?;
    rt.access()
        .map_err(access_error)?
        .disconnect(repo_actor)
        .await
        .map_err(access_error)?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(post,path="/api/projects/repositories",operation_id="listImportableRepositories",request_body=RepositoryPageBody,params(("Origin"=String,Header,description="Exact frontend Origin"),("X-CSRF-Token"=String,Header,description="Session CSRF token")),security(("sessionCookie"=[])),responses((status=200,body=RepositoryPageView),(status=401,body=ApiError),(status=403,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError),(status=400,body=ApiError)))]
async fn repositories(
    State(s): State<StateData>,
    h: HeaderMap,
    body: Result<Json<RepositoryPageBody>, JsonRejection>,
) -> Result<Json<RepositoryPageView>, Response> {
    let (_, actor) = s.actor(&h, Some(true)).await?;
    let Json(body) = body.map_err(|_| access_error(RepositoryAccessError::InvalidInput))?;
    let rt = s.runtime().map_err(access_error)?;
    let (_, repo_actor) = rt.actor(actor).await.map_err(access_error)?;
    let page = rt
        .access()
        .map_err(access_error)?
        .repositories(repo_actor, body.page)
        .await
        .map_err(access_error)?;
    let metadata = page
        .items
        .iter()
        .map(|repository| factory_core::projects::RepositoryMetadata {
            repository_id: repository.id,
            full_name: repository.full_name.clone(),
        })
        .collect::<Vec<_>>();
    let imported: HashSet<_> = rt
        .projects
        .sync_repository_metadata(actor, &metadata)
        .await
        .map_err(project_error)?
        .into_iter()
        .collect();
    Ok(Json(importable(page, &imported)))
}
fn importable(
    page: factory_core::identity::AccessibleRepositoryPage,
    imported: &HashSet<i64>,
) -> RepositoryPageView {
    RepositoryPageView {
        items: page
            .items
            .into_iter()
            .filter(|r| !imported.contains(&r.id))
            .map(Into::into)
            .collect(),
        next_page: page.next_page,
    }
}
#[utoipa::path(get,path="/api/projects",operation_id="listProjects",params(("before"=Option<Uuid>,Query)),security(("sessionCookie"=[])),responses((status=200,body=ProjectPageView),(status=401,body=ApiError),(status=503,body=ApiError),(status=400,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError)))]
async fn list(
    State(s): State<StateData>,
    h: HeaderMap,
    q: Result<Query<ProjectQuery>, QueryRejection>,
) -> Result<Json<ProjectPageView>, Response> {
    let (_, actor) = s.actor(&h, None).await?;
    let Query(q) = q.map_err(|_| project_error(ProjectError::Invalid))?;
    let rt = s.runtime().map_err(access_error)?;
    let page = rt
        .projects
        .list(actor, q.before)
        .await
        .map_err(project_error)?;
    Ok(Json(ProjectPageView {
        items: page.items.into_iter().map(|p| rt.view(p)).collect(),
        next_cursor: page.next_cursor,
    }))
}
#[utoipa::path(post,path="/api/projects",operation_id="importProject",request_body=ImportBody,security(("sessionCookie"=[])),params(("Origin"=String,Header,description="Exact frontend Origin"),("X-CSRF-Token"=String,Header,description="Session CSRF token")),responses((status=202,body=ProjectView),(status=400,body=ApiError),(status=401,body=ApiError),(status=403,body=ApiError),(status=409,body=ApiError),(status=503,body=ApiError),(status=429,body=ApiError)))]
async fn import(
    State(s): State<StateData>,
    h: HeaderMap,
    body: Result<Json<ImportBody>, JsonRejection>,
) -> Result<(StatusCode, Json<ProjectView>), Response> {
    let (_, actor) = s.actor(&h, Some(true)).await?;
    let Json(body) = body.map_err(|_| project_error(ProjectError::Invalid))?;
    let rt = s.runtime().map_err(access_error)?;
    let project = rt
        .import(actor, body.repository_id, body.page)
        .await
        .map_err(access_error)?;
    let project = rt
        .projects
        .get(actor, project.id)
        .await
        .map_err(project_error)?;
    Ok((StatusCode::ACCEPTED, Json(rt.view(project))))
}
#[utoipa::path(get,path="/api/projects/recoverable",operation_id="listRecoverableImports",security(("sessionCookie"=[])),responses((status=200,body=[RecoverableImportView]),(status=401,body=ApiError),(status=503,body=ApiError),(status=400,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError)))]
async fn recoverable(
    State(s): State<StateData>,
    h: HeaderMap,
) -> Result<Json<Vec<RecoverableImportView>>, Response> {
    let (_, actor) = s.actor(&h, None).await?;
    let items = s
        .runtime()
        .map_err(access_error)?
        .projects
        .recoverable(actor)
        .await
        .map_err(project_error)?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}
#[utoipa::path(get,path="/api/projects/{id}",operation_id="getProject",params(("id"=Uuid,Path)),security(("sessionCookie"=[])),responses((status=200,body=ProjectView),(status=401,body=ApiError),(status=404,body=ApiError),(status=503,body=ApiError),(status=400,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError)))]
async fn get(
    State(s): State<StateData>,
    h: HeaderMap,
    id: Result<Path<Uuid>, PathRejection>,
) -> Result<Json<ProjectView>, Response> {
    let (_, actor) = s.actor(&h, None).await?;
    let Path(id) = id.map_err(|_| project_error(ProjectError::Invalid))?;
    let rt = s.runtime().map_err(access_error)?;
    let project = rt.projects.get(actor, id).await.map_err(project_error)?;
    Ok(Json(rt.view(project)))
}
#[utoipa::path(delete,path="/api/projects/{id}",operation_id="deleteProject",params(("id"=Uuid,Path),("Origin"=String,Header,description="Exact frontend Origin"),("X-CSRF-Token"=String,Header,description="Session CSRF token")),security(("sessionCookie"=[])),responses((status=204),(status=400,body=ApiError),(status=401,body=ApiError),(status=403,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn delete(
    State(s): State<StateData>,
    h: HeaderMap,
    id: Result<Path<Uuid>, PathRejection>,
    body: axum::body::Bytes,
) -> Result<StatusCode, Response> {
    let (_, actor) = s.actor(&h, Some(false)).await?;
    if !body.is_empty() {
        return Err(project_error(ProjectError::Invalid));
    }
    let Path(id) = id.map_err(|_| project_error(ProjectError::Invalid))?;
    let rt = s.runtime().map_err(access_error)?;
    // Keep the submitted deletion alive after a browser disconnect.
    tokio::spawn(async move { rt.delete(actor, id).await })
        .await
        .map_err(|_| project_error(ProjectError::Unavailable))?
        .map_err(project_error)?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/api/projects/{id}/verification",operation_id="verifyProjectRepository",params(("id"=Uuid,Path)),security(("sessionCookie"=[])),responses((status=200,body=ProjectVerificationView),(status=400,body=ApiError),(status=401,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn verification(
    State(s): State<StateData>,
    h: HeaderMap,
    id: Result<Path<Uuid>, PathRejection>,
) -> Result<Json<ProjectVerificationView>, Response> {
    let (_, actor) = s.actor(&h, None).await?;
    let Path(id) = id.map_err(|_| project_error(ProjectError::Invalid))?;
    let rt = s.runtime().map_err(access_error)?;
    let project = rt.projects.get(actor, id).await.map_err(project_error)?;
    let (_, repo_actor) = rt.actor(actor).await.map_err(access_error)?;
    let status = rt
        .access()
        .map_err(access_error)?
        .verify_clone_origin(repo_actor, &project)
        .await
        .map_err(access_error)?;
    Ok(Json(ProjectVerificationView {
        status: match status {
            RepositoryOriginVerification::Matched => ProjectVerificationStatus::Matched,
            RepositoryOriginVerification::Changed => ProjectVerificationStatus::Changed,
        },
    }))
}
#[utoipa::path(post,path="/api/projects/{id}/recover",operation_id="recoverProjectImport",params(("id"=Uuid,Path),("Origin"=String,Header,description="Exact frontend Origin"),("X-CSRF-Token"=String,Header,description="Session CSRF token")),security(("sessionCookie"=[])),responses((status=204),(status=401,body=ApiError),(status=403,body=ApiError),(status=409,body=ApiError),(status=503,body=ApiError),(status=400,body=ApiError),(status=429,body=ApiError)))]
async fn recover(
    State(s): State<StateData>,
    h: HeaderMap,
    id: Result<Path<Uuid>, PathRejection>,
    body: axum::body::Bytes,
) -> Result<StatusCode, Response> {
    let (_, actor) = s.actor(&h, Some(false)).await?;
    if !body.is_empty() {
        return Err(project_error(ProjectError::Invalid));
    }
    let Path(id) = id.map_err(|_| project_error(ProjectError::Invalid))?;
    let rt = s.runtime().map_err(access_error)?;
    let env = rt
        .environments
        .get(actor)
        .await
        .map_err(|_| project_error(ProjectError::Unavailable))?
        .ok_or_else(|| project_error(ProjectError::NotFound))?;
    if !rt.environments.can_report(Some(&env))
        || env.desired_state != DesiredState::Stopped
        || env.observed_state != ObservedState::Stopped
    {
        return Err(project_error(ProjectError::Conflict));
    }
    let observed = env
        .observed_at
        .ok_or_else(|| project_error(ProjectError::Conflict))?;
    rt.store
        .recover_stopped(actor, env.id, env.generation, observed, id)
        .await
        .map_err(project_error)?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(post,path="/api/projects/{id}/retry",operation_id="retryProjectImport",params(("id"=Uuid,Path),("Origin"=String,Header,description="Exact frontend Origin"),("X-CSRF-Token"=String,Header,description="Session CSRF token")),security(("sessionCookie"=[])),responses((status=202,body=ProjectView),(status=401,body=ApiError),(status=403,body=ApiError),(status=409,body=ApiError),(status=503,body=ApiError),(status=400,body=ApiError),(status=429,body=ApiError)))]
async fn retry(
    State(s): State<StateData>,
    h: HeaderMap,
    id: Result<Path<Uuid>, PathRejection>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<ProjectView>), Response> {
    let (_, actor) = s.actor(&h, Some(false)).await?;
    if !body.is_empty() {
        return Err(project_error(ProjectError::Invalid));
    }
    let Path(id) = id.map_err(|_| project_error(ProjectError::Invalid))?;
    let rt = s.runtime().map_err(access_error)?;
    let project = rt.retry(actor, id).await.map_err(project_error)?;
    let project = rt
        .projects
        .get(actor, project.id)
        .await
        .map_err(project_error)?;
    Ok((StatusCode::ACCEPTED, Json(rt.view(project))))
}

#[cfg(test)]
mod tests;
