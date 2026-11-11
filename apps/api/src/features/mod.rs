mod contracts;

use self::contracts::*;
use crate::{
    auth::{AuthError, AuthState},
    contracts::ApiError,
    conversations::contracts::RunView,
};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use factory_application::features::{FeatureCoordinator, FeatureDeliveryService};
use factory_core::{
    environments::EnvironmentActor,
    features::{DeliveryStatus, FeatureError},
    identity::IdentityError,
};
use std::{future::Future, sync::Arc};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct FeatureRuntime {
    pub coordinator: Arc<FeatureCoordinator>,
    pub delivery: Option<Arc<FeatureDeliveryService>>,
}

#[derive(Clone)]
struct StateData {
    auth: Option<AuthState>,
    runtime: Option<FeatureRuntime>,
}

impl StateData {
    async fn actor(
        &self,
        headers: &HeaderMap,
        mutation: bool,
    ) -> Result<EnvironmentActor, Response> {
        let auth = self
            .auth
            .as_ref()
            .ok_or_else(|| AuthError::from(IdentityError::Unavailable).into_response())?;
        auth.check_quota(false)
            .map_err(IntoResponse::into_response)?;
        let a = auth
            .authenticate(headers)
            .await
            .map_err(IntoResponse::into_response)?;
        if mutation {
            auth.authorize_mutation(headers, &a, true)
                .map_err(IntoResponse::into_response)?;
        }
        Ok(EnvironmentActor {
            user_id: a.session.user.id,
            session_hash: a.token_hash,
        })
    }

    fn runtime(&self) -> Result<&FeatureRuntime, FeatureError> {
        self.runtime.as_ref().ok_or(FeatureError::Unavailable)
    }
}

pub(crate) fn documented_router(
    auth: Option<AuthState>,
    runtime: Option<FeatureRuntime>,
) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(get))
        .routes(routes!(discuss))
        .routes(routes!(approve))
        .routes(routes!(reopen))
        .routes(routes!(resume))
        .routes(routes!(diff))
        .routes(routes!(review))
        .routes(routes!(draft))
        .layer(DefaultBodyLimit::max(80 * 1024))
        .with_state(StateData { auth, runtime })
}

pub fn router(
    auth: AuthState,
    coordinator: Arc<FeatureCoordinator>,
    delivery: Option<Arc<FeatureDeliveryService>>,
) -> Router {
    documented_router(
        Some(auth),
        Some(FeatureRuntime {
            coordinator,
            delivery,
        }),
    )
    .into()
}

fn error(e: FeatureError) -> Response {
    let (status, code) = match e {
        FeatureError::Unauthorized => (StatusCode::UNAUTHORIZED, "session-required"),
        FeatureError::NotFound => (StatusCode::NOT_FOUND, "feature-not-found"),
        FeatureError::Invalid => (StatusCode::BAD_REQUEST, "invalid-input"),
        FeatureError::Conflict => (StatusCode::CONFLICT, "feature-conflict"),
        FeatureError::RepositoryAuthorizationRequired => (
            StatusCode::CONFLICT,
            "feature-repository-authorization-required",
        ),
        FeatureError::ExecutionUncertain => (StatusCode::CONFLICT, "feature-execution-uncertain"),
        FeatureError::ReviewBlocked => (StatusCode::CONFLICT, "feature-review-blocked"),
        FeatureError::StaleSnapshot => (StatusCode::CONFLICT, "feature-stale-snapshot"),
        FeatureError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
    };
    (
        status,
        Json(ApiError {
            code: code.into(),
            message: e.to_string(),
            request_id: Uuid::new_v4(),
        }),
    )
        .into_response()
}

fn feature_id(path: Result<Path<Uuid>, PathRejection>) -> Result<Uuid, FeatureError> {
    path.map(|Path(id)| id).map_err(|_| FeatureError::Invalid)
}

fn payload<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, FeatureError> {
    body.map(|Json(body)| body)
        .map_err(|_| FeatureError::Invalid)
}

/// The client may disconnect after the command has been claimed. Let the
/// command finish recording its receipt, so the same operation can be replayed.
async fn command<T, V>(
    work: impl Future<Output = Result<T, FeatureError>> + Send + 'static,
) -> Result<Json<V>, Response>
where
    T: Into<V> + Send + 'static,
{
    let value = tokio::spawn(work)
        .await
        .map_err(|_| error(FeatureError::Unavailable))?
        .map_err(error)?;
    Ok(Json(value.into()))
}

#[utoipa::path(get,path="/api/features",operation_id="listFeatures",params(("projectId"=Option<Uuid>,Query),("before"=Option<Uuid>,Query)),security(("sessionCookie"=[])),responses((status=200,body=FeaturePageView),(status=400,body=ApiError),(status=401,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn list(
    State(state): State<StateData>,
    query: Result<Query<FeatureListQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Result<Json<FeaturePageView>, Response> {
    let actor = state.actor(&headers, false).await?;
    let Query(query) = query.map_err(|_| error(FeatureError::Invalid))?;
    let result = state
        .runtime()
        .map_err(error)?
        .coordinator
        .list(actor, query.project_id, query.before)
        .await
        .map_err(error)?;
    Ok(Json(result.into()))
}

#[utoipa::path(post,path="/api/features",operation_id="createFeature",request_body=CreateFeatureBody,params(("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=200,body=FeatureView),(status=400,body=ApiError),(status=401,body=ApiError),(status=403,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn create(
    State(state): State<StateData>,
    headers: HeaderMap,
    body: Result<Json<CreateFeatureBody>, JsonRejection>,
) -> Result<Json<FeatureView>, Response> {
    let actor = state.actor(&headers, true).await?;
    let body = payload(body).map_err(error)?;
    let coordinator = state.runtime().map_err(error)?.coordinator.clone();
    command(async move {
        coordinator
            .create(
                actor,
                body.operation_id,
                body.project_id,
                body.provider.id().to_owned(),
                body.title,
                body.locale.language(),
            )
            .await
    })
    .await
}

#[utoipa::path(get,path="/api/features/{id}",operation_id="getFeature",params(("id"=Uuid,Path)),security(("sessionCookie"=[])),responses((status=200,body=FeatureView),(status=400,body=ApiError),(status=401,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn get(
    State(state): State<StateData>,
    path: Result<Path<Uuid>, PathRejection>,
    headers: HeaderMap,
) -> Result<Json<FeatureView>, Response> {
    let actor = state.actor(&headers, false).await?;
    let id = feature_id(path).map_err(error)?;
    let coordinator = &state.runtime().map_err(error)?.coordinator;
    let result = coordinator.get(actor, id).await.map_err(error)?;
    let driver_active = coordinator.is_driving(id);
    let can_continue = !driver_active && coordinator.can_continue(&result);
    Ok(Json(FeatureView::from_state(
        result,
        can_continue,
        driver_active,
        |attempt_id| coordinator.is_attempt_locally_active(id, attempt_id),
    )))
}

#[utoipa::path(post,path="/api/features/{id}/runs",operation_id="discussFeature",request_body=FeatureTurnBody,params(("id"=Uuid,Path),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=200,body=RunView),(status=400,body=ApiError),(status=401,body=ApiError),(status=403,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn discuss(
    State(state): State<StateData>,
    path: Result<Path<Uuid>, PathRejection>,
    headers: HeaderMap,
    body: Result<Json<FeatureTurnBody>, JsonRejection>,
) -> Result<Json<RunView>, Response> {
    let actor = state.actor(&headers, true).await?;
    let id = feature_id(path).map_err(error)?;
    let body = payload(body).map_err(error)?;
    let coordinator = state.runtime().map_err(error)?.coordinator.clone();
    command(async move {
        coordinator
            .discuss(actor, id, body.operation_id, body.input)
            .await
    })
    .await
}

#[utoipa::path(post,path="/api/features/{id}/approve",operation_id="approveFeature",request_body=FeatureRevisionBody,params(("id"=Uuid,Path),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=200,body=FeatureView),(status=400,body=ApiError),(status=401,body=ApiError),(status=403,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn approve(
    State(state): State<StateData>,
    path: Result<Path<Uuid>, PathRejection>,
    headers: HeaderMap,
    body: Result<Json<FeatureRevisionBody>, JsonRejection>,
) -> Result<Json<FeatureView>, Response> {
    let actor = state.actor(&headers, true).await?;
    let id = feature_id(path).map_err(error)?;
    let body = payload(body).map_err(error)?;
    let coordinator = state.runtime().map_err(error)?.coordinator.clone();
    command(async move {
        coordinator
            .approve(actor, id, body.operation_id, body.into_revision())
            .await
    })
    .await
}

#[utoipa::path(post,path="/api/features/{id}/reopen",operation_id="reopenFeatureGate",request_body=FeatureRevisionBody,params(("id"=Uuid,Path),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=200,body=FeatureView),(status=400,body=ApiError),(status=401,body=ApiError),(status=403,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn reopen(
    State(state): State<StateData>,
    path: Result<Path<Uuid>, PathRejection>,
    headers: HeaderMap,
    body: Result<Json<FeatureRevisionBody>, JsonRejection>,
) -> Result<Json<FeatureView>, Response> {
    let actor = state.actor(&headers, true).await?;
    let id = feature_id(path).map_err(error)?;
    let body = payload(body).map_err(error)?;
    let coordinator = state.runtime().map_err(error)?.coordinator.clone();
    command(async move {
        coordinator
            .reopen_gate(actor, id, body.operation_id, body.into_revision())
            .await
    })
    .await
}

#[utoipa::path(post,path="/api/features/{id}/resume",operation_id="resumeFeatureAttempt",request_body=FeatureResumeBody,params(("id"=Uuid,Path),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=200,body=FeatureView),(status=400,body=ApiError),(status=401,body=ApiError),(status=403,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn resume(
    State(state): State<StateData>,
    path: Result<Path<Uuid>, PathRejection>,
    headers: HeaderMap,
    body: Result<Json<FeatureResumeBody>, JsonRejection>,
) -> Result<Json<FeatureView>, Response> {
    let actor = state.actor(&headers, true).await?;
    let id = feature_id(path).map_err(error)?;
    let body = payload(body).map_err(error)?;
    let coordinator = state.runtime().map_err(error)?.coordinator.clone();
    command(async move {
        match body.attempt_id {
            Some(attempt_id) => {
                coordinator
                    .resume(actor, id, body.operation_id, attempt_id)
                    .await
            }
            None => {
                coordinator
                    .continue_feature(actor, id, body.operation_id)
                    .await
            }
        }
    })
    .await
}

#[utoipa::path(get,path="/api/features/{id}/diff",operation_id="getFeatureDiff",params(("id"=Uuid,Path)),security(("sessionCookie"=[])),responses((status=200,body=FeatureDiffView),(status=400,body=ApiError),(status=401,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn diff(
    State(state): State<StateData>,
    path: Result<Path<Uuid>, PathRejection>,
    headers: HeaderMap,
) -> Result<Json<FeatureDiffView>, Response> {
    let actor = state.actor(&headers, false).await?;
    let id = feature_id(path).map_err(error)?;
    let runtime = state.runtime().map_err(error)?;
    if let Some(delivery) = &runtime.delivery {
        if let Some(diff) = delivery
            .published_stale_diff(actor, id)
            .await
            .map_err(error)?
        {
            return Ok(Json(diff.into()));
        }
    } else {
        let feature = runtime.coordinator.get(actor, id).await.map_err(error)?;
        if feature
            .deliveries
            .iter()
            .any(|receipt| matches!(receipt.status, DeliveryStatus::PublishedStale { .. }))
        {
            return Err(error(FeatureError::Unavailable));
        }
    }
    let result = runtime
        .coordinator
        .reviewed_diff(actor, id)
        .await
        .map_err(error)?;
    Ok(Json(result.into()))
}

#[utoipa::path(post,path="/api/features/{id}/review",operation_id="extendFeatureReview",request_body=FeatureSnapshotBody,params(("id"=Uuid,Path),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=200,body=FeatureView),(status=400,body=ApiError),(status=401,body=ApiError),(status=403,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn review(
    State(state): State<StateData>,
    path: Result<Path<Uuid>, PathRejection>,
    headers: HeaderMap,
    body: Result<Json<FeatureSnapshotBody>, JsonRejection>,
) -> Result<Json<FeatureView>, Response> {
    let actor = state.actor(&headers, true).await?;
    let id = feature_id(path).map_err(error)?;
    let body = payload(body).map_err(error)?;
    let runtime = state.runtime().map_err(error)?;
    let coordinator = runtime.coordinator.clone();
    let delivery = runtime.delivery.clone();
    command(async move {
        if let Some(delivery) = delivery {
            delivery
                .refresh_stale_comparison(actor, id, body.snapshot_id)
                .await?;
        }
        coordinator
            .authorize_additional_review(actor, id, body.operation_id, body.snapshot_id)
            .await
    })
    .await
}

#[utoipa::path(post,path="/api/features/{id}/draft",operation_id="publishFeatureDraft",request_body=FeatureSnapshotBody,params(("id"=Uuid,Path),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=200,body=FeatureView),(status=400,body=ApiError),(status=401,body=ApiError),(status=403,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn draft(
    State(state): State<StateData>,
    path: Result<Path<Uuid>, PathRejection>,
    headers: HeaderMap,
    body: Result<Json<FeatureSnapshotBody>, JsonRejection>,
) -> Result<Json<FeatureView>, Response> {
    let actor = state.actor(&headers, true).await?;
    let id = feature_id(path).map_err(error)?;
    let body = payload(body).map_err(error)?;
    let delivery = state
        .runtime()
        .map_err(error)?
        .delivery
        .clone()
        .ok_or_else(|| error(FeatureError::Unavailable))?;
    command(async move {
        delivery
            .publish(actor, id, body.operation_id, body.snapshot_id)
            .await
    })
    .await
}
