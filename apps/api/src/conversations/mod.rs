pub(crate) mod contracts;
pub mod runtime;
use self::{contracts::*, runtime::ConversationRuntime};
use crate::{
    auth::{AuthError, AuthState, Authenticated},
    contracts::ApiError,
};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use factory_core::{conversations::ConversationError, environments::EnvironmentActor};
use futures_util::stream;
use std::{convert::Infallible, sync::Arc, time::Duration};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

#[derive(Clone)]
struct StateData {
    auth: Option<AuthState>,
    runtime: Option<Arc<ConversationRuntime>>,
}
impl StateData {
    async fn authenticate(
        &self,
        headers: &HeaderMap,
        mutation: Option<bool>,
    ) -> Result<(AuthState, EnvironmentActor), Response> {
        let auth = self.auth.clone().ok_or_else(|| {
            AuthError::from(factory_core::identity::IdentityError::Unavailable).into_response()
        })?;
        auth.check_quota(false)
            .map_err(IntoResponse::into_response)?;
        let a = auth
            .authenticate(headers)
            .await
            .map_err(IntoResponse::into_response)?;
        if let Some(body) = mutation {
            auth.authorize_mutation(headers, &a, body)
                .map_err(IntoResponse::into_response)?;
        }
        Ok((auth, actor(&a)))
    }
    fn runtime(&self) -> Result<Arc<ConversationRuntime>, ConversationError> {
        self.runtime.clone().ok_or(ConversationError::Unavailable)
    }
}
fn actor(a: &Authenticated) -> EnvironmentActor {
    EnvironmentActor {
        user_id: a.session.user.id,
        session_hash: a.token_hash,
    }
}
pub(crate) fn documented_router(
    auth: Option<AuthState>,
    runtime: Option<Arc<ConversationRuntime>>,
) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list, create))
        .routes(routes!(history))
        .routes(routes!(models))
        .routes(routes!(recoverable))
        .routes(routes!(recover_model_probe))
        .routes(routes!(start))
        .routes(routes!(cancel))
        .routes(routes!(answer))
        .routes(routes!(events))
        .routes(routes!(recover))
        .layer(DefaultBodyLimit::max(80 * 1024))
        .with_state(StateData { auth, runtime })
}
#[allow(dead_code)]
pub fn router(auth: AuthState, runtime: Option<Arc<ConversationRuntime>>) -> Router {
    documented_router(Some(auth), runtime).into()
}
fn error(e: ConversationError) -> Response {
    let status = match e {
        ConversationError::Unauthorized => StatusCode::UNAUTHORIZED,
        ConversationError::NotFound => StatusCode::NOT_FOUND,
        ConversationError::Conflict | ConversationError::ContextLimit => StatusCode::CONFLICT,
        ConversationError::Invalid => StatusCode::BAD_REQUEST,
        ConversationError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        status,
        Json(ApiError {
            code: format!("{e:?}"),
            message: e.to_string(),
            request_id: Uuid::new_v4(),
        }),
    )
        .into_response()
}
#[utoipa::path(get,path="/api/conversations",operation_id="listConversations",params(("before"=Option<Uuid>,Query)),security(("sessionCookie"=[])),responses((status=200,body=ConversationPageView),(status=400,body=ApiError),(status=401,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn list(
    State(s): State<StateData>,
    query: Result<Query<ConversationListQuery>, QueryRejection>,
    h: HeaderMap,
) -> Result<Json<ConversationPageView>, Response> {
    let (_, a) = s.authenticate(&h, None).await?;
    let Query(query) = query.map_err(|_| error(ConversationError::Invalid))?;
    let page = s
        .runtime()
        .map_err(error)?
        .service
        .list(a, query.before)
        .await
        .map_err(error)?;
    Ok(Json(ConversationPageView {
        items: page.items.into_iter().map(Into::into).collect(),
        next_cursor: page.next_cursor,
    }))
}
#[utoipa::path(post,path="/api/conversations",operation_id="createConversation",request_body=Option<CreateConversationBody>,params(("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=200,body=ConversationView),(status=400,description="Invalid path or body",body=ApiError),(status=401,description="Active session required",body=ApiError),(status=403,description="Exact Origin or CSRF rejected",body=ApiError),(status=429,description="Request rate limit",body=ApiError),(status=503,description="Service unavailable",body=ApiError),(status=409,body=ApiError)))]
async fn create(
    State(s): State<StateData>,
    h: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<ConversationView>, Response> {
    let (_, a) = s.authenticate(&h, Some(!body.is_empty())).await?;
    let project_id = if body.is_empty() {
        None
    } else {
        serde_json::from_slice::<CreateConversationBody>(&body)
            .map_err(|_| error(ConversationError::Invalid))?
            .project_id
    };
    Ok(Json(
        s.runtime()
            .map_err(error)?
            .service
            .create_in_project(a, project_id)
            .await
            .map_err(error)?
            .into(),
    ))
}
#[utoipa::path(get,path="/api/conversations/{id}",operation_id="getConversation",params(("id"=Uuid,Path)),security(("sessionCookie"=[])),responses((status=200,body=HistoryView),(status=400,body=ApiError),(status=401,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn history(
    State(s): State<StateData>,
    path: Result<Path<Uuid>, PathRejection>,
    h: HeaderMap,
) -> Result<Json<HistoryView>, Response> {
    let (_, a) = s.authenticate(&h, None).await?;
    let Path(id) = path.map_err(|_| error(ConversationError::Invalid))?;
    let h = s
        .runtime()
        .map_err(error)?
        .service
        .history(a, id)
        .await
        .map_err(error)?;
    Ok(Json(
        h.try_into()
            .map_err(|_| error(ConversationError::Unavailable))?,
    ))
}
#[utoipa::path(get,path="/api/conversations/models",operation_id="listConversationModels",security(("sessionCookie"=[])),responses((status=200,body=[ModelView]),(status=401,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn models(
    State(s): State<StateData>,
    h: HeaderMap,
) -> Result<Json<Vec<ModelView>>, Response> {
    let (auth, a) = s.authenticate(&h, None).await?;
    let rt = s.runtime().map_err(error)?;
    // Detached HTTP requests must not abandon a process after Docker may have started it.
    let models = tokio::spawn(async move { rt.models(a).await })
        .await
        .map_err(|_| error(ConversationError::Unavailable))?
        .map_err(error)?;
    auth.authenticate(&h)
        .await
        .map_err(IntoResponse::into_response)?;
    Ok(Json(
        models
            .into_iter()
            .map(|m| ModelView {
                id: m.id,
                name: m.name,
                resolved_model: m.resolved_model,
                effort_levels: m.effort_levels,
                default_effort: m.default_effort,
            })
            .collect(),
    ))
}
#[utoipa::path(post,path="/api/conversations/{id}/runs",operation_id="startConversationRun",params(("id"=Uuid,Path),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),request_body=StartBody,security(("sessionCookie"=[])),responses((status=200,body=RunView),(status=400,description="Invalid path or body",body=ApiError),(status=401,description="Active session required",body=ApiError),(status=403,description="Exact Origin or CSRF rejected",body=ApiError),(status=429,description="Request rate limit",body=ApiError),(status=503,description="Service unavailable",body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError)))]
async fn start(
    State(s): State<StateData>,
    path: Result<Path<Uuid>, PathRejection>,
    h: HeaderMap,
    body: Result<Json<StartBody>, JsonRejection>,
) -> Result<Json<RunView>, Response> {
    let (_, a) = s.authenticate(&h, Some(true)).await?;
    let Path(id) = path.map_err(|_| error(ConversationError::Invalid))?;
    let Json(b) = body.map_err(|_| error(ConversationError::Invalid))?;
    let rt = s.runtime().map_err(error)?;
    let task = tokio::spawn(async move {
        rt.begin(
            a,
            id,
            b.operation_id,
            b.model,
            b.effort,
            b.input,
            b.attachment_ids,
        )
        .await
    });
    let run = task
        .await
        .map_err(|_| error(ConversationError::Unavailable))?
        .map_err(error)?;
    Ok(Json(run.into()))
}
#[utoipa::path(post,path="/api/conversations/{id}/runs/{run}/cancel",operation_id="cancelConversationRun",params(("id"=Uuid,Path),("run"=Uuid,Path),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=204),(status=400,description="Invalid path or body",body=ApiError),(status=401,description="Active session required",body=ApiError),(status=403,description="Exact Origin or CSRF rejected",body=ApiError),(status=429,description="Request rate limit",body=ApiError),(status=503,description="Service unavailable",body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError)))]
async fn cancel(
    State(s): State<StateData>,
    path: Result<Path<(Uuid, Uuid)>, PathRejection>,
    h: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, Response> {
    let (_, a) = s.authenticate(&h, Some(false)).await?;
    let Path((id, run)) = path.map_err(|_| error(ConversationError::Invalid))?;
    if !body.is_empty() {
        return Err(error(ConversationError::Invalid));
    }
    s.runtime()
        .map_err(error)?
        .interrupt(a, id, run)
        .await
        .map_err(error)?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(post,path="/api/conversations/{id}/runs/{run}/questions/{request}",operation_id="answerConversationQuestion",params(("id"=Uuid,Path),("run"=Uuid,Path),("request"=Uuid,Path),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),request_body=AnswerBody,security(("sessionCookie"=[])),responses((status=204),(status=400,description="Invalid path or body",body=ApiError),(status=401,description="Active session required",body=ApiError),(status=403,description="Exact Origin or CSRF rejected",body=ApiError),(status=429,description="Request rate limit",body=ApiError),(status=503,description="Service unavailable",body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError)))]
async fn answer(
    State(s): State<StateData>,
    path: Result<Path<(Uuid, Uuid, Uuid)>, PathRejection>,
    h: HeaderMap,
    body: Result<Json<AnswerBody>, JsonRejection>,
) -> Result<StatusCode, Response> {
    let (_, a) = s.authenticate(&h, Some(true)).await?;
    let Path((id, run, request)) = path.map_err(|_| error(ConversationError::Invalid))?;
    let Json(b) = body.map_err(|_| error(ConversationError::Invalid))?;
    let rt = s.runtime().map_err(error)?;
    tokio::spawn(async move {
        rt.answer(
            a,
            id,
            run,
            request,
            b.operation_id,
            b.answers
                .into_iter()
                .map(|a| factory_core::conversations::execution::QuestionAnswer {
                    id: a.id,
                    value: a.value,
                })
                .collect(),
        )
        .await
    })
    .await
    .map_err(|_| error(ConversationError::Unavailable))?
    .map_err(error)?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/api/conversations/recoverable",operation_id="listRecoverableRuns",security(("sessionCookie"=[])),responses((status=200,body=[RecoverableRunView]),(status=401,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn recoverable(
    State(s): State<StateData>,
    h: HeaderMap,
) -> Result<Json<Vec<RecoverableRunView>>, Response> {
    let (auth, actor) = s.authenticate(&h, None).await?;
    let runs = s
        .runtime()
        .map_err(error)?
        .service
        .recoverable_stopped(actor)
        .await
        .map_err(error)?;
    auth.authenticate(&h)
        .await
        .map_err(IntoResponse::into_response)?;
    Ok(Json(
        runs.into_iter()
            .map(|run| RecoverableRunView {
                conversation_id: run.conversation_id,
                run_id: run.run_id,
                launch_settled: run.launch_settled,
            })
            .collect(),
    ))
}
#[utoipa::path(post,path="/api/conversations/model-probe/recover",operation_id="recoverModelProbe",params(("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=204),(status=400,description="Invalid path or body",body=ApiError),(status=401,description="Active session required",body=ApiError),(status=403,description="Exact Origin or CSRF rejected",body=ApiError),(status=429,description="Request rate limit",body=ApiError),(status=503,description="Service unavailable",body=ApiError),(status=409,body=ApiError)))]
async fn recover_model_probe(
    State(s): State<StateData>,
    h: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, Response> {
    let (_, actor) = s.authenticate(&h, Some(false)).await?;
    if !body.is_empty() {
        return Err(error(ConversationError::Invalid));
    }
    s.runtime()
        .map_err(error)?
        .service
        .recover_model_probe_stopped(actor)
        .await
        .map_err(error)?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(post,path="/api/conversations/{id}/runs/{run}/recover",operation_id="recoverConversationRun",params(("id"=Uuid,Path),("run"=Uuid,Path),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=204),(status=400,description="Invalid path or body",body=ApiError),(status=401,description="Active session required",body=ApiError),(status=403,description="Exact Origin or CSRF rejected",body=ApiError),(status=429,description="Request rate limit",body=ApiError),(status=503,description="Service unavailable",body=ApiError),(status=409,body=ApiError)))]
async fn recover(
    State(s): State<StateData>,
    path: Result<Path<(Uuid, Uuid)>, PathRejection>,
    h: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, Response> {
    let (_, a) = s.authenticate(&h, Some(false)).await?;
    let Path((id, run)) = path.map_err(|_| error(ConversationError::Invalid))?;
    if !body.is_empty() {
        return Err(error(ConversationError::Invalid));
    }
    let rt = s.runtime().map_err(error)?;
    if rt.has_active(a.user_id).await {
        return Err(error(ConversationError::Conflict));
    }
    let _ = id;
    rt.service.recover_stopped(a, run).await.map_err(error)?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/api/conversations/{id}/runs/{run}/events",operation_id="streamConversationRun",params(("id"=Uuid,Path),("run"=Uuid,Path),("Origin"=Option<String>,Header,description="Exact configured frontend Origin required unless the web and API share an origin and the browser sends Sec-Fetch-Site: same-origin")),security(("sessionCookie"=[])),responses((status=200,description="Authorized bounded event stream",content_type="text/event-stream"),(status=400,body=ApiError),(status=401,body=ApiError),(status=403,body=ApiError),(status=404,body=ApiError),(status=409,body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn events(
    State(s): State<StateData>,
    path: Result<Path<(Uuid, Uuid)>, PathRejection>,
    h: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, Response> {
    let (auth, a) = s.authenticate(&h, None).await?;
    let Path((id, run)) = path.map_err(|_| error(ConversationError::Invalid))?;
    auth.authorize_event_stream_origin(&h)
        .map_err(IntoResponse::into_response)?;
    let rt = s.runtime().map_err(error)?;
    // Subscribe before a fresh authentication so revocation cannot be missed while attaching.
    let revocations = auth.service().subscribe_revocations();
    let verified = auth
        .authenticate(&h)
        .await
        .map_err(IntoResponse::into_response)?;
    if verified.token_hash != a.session_hash || verified.session.user.id != a.user_id {
        return Err(error(ConversationError::Unauthorized));
    }
    let expires_at = verified.session.expires_at;
    let rx = rt.subscribe(a, id, run).await.map_err(error)?;
    let next_check = tokio::time::Instant::now() + Duration::from_secs(5);
    let st = stream::unfold(
        (rx, revocations, auth, h, rt, a, next_check, expires_at),
        move |(mut rx, mut revocations, auth, h, rt, a, mut next_check, expires_at)| async move {
            // Drain queued revocations before delivering each fragment; a lagged receiver fails closed.
            loop {
                match revocations.try_recv() {
                    Ok(event) if event.matches(a.user_id, a.session_hash) => return None,
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                    Err(_) => return None,
                }
            }
            if chrono::Utc::now() >= expires_at {
                return None;
            }
            let remaining = (expires_at - chrono::Utc::now())
                .to_std()
                .unwrap_or(Duration::ZERO);
            let expiry = tokio::time::Instant::now() + remaining;
            let item = tokio::select! {
                biased;
                _ = revocation(&mut revocations, a) => return None,
                _ = tokio::time::sleep_until(expiry) => return None,
                _ = tokio::time::sleep_until(next_check) => {
                    if auth.authenticate(&h).await.is_err() || rt.service.current_environment(a).await.is_err() { return None; }
                    next_check = tokio::time::Instant::now() + Duration::from_secs(5);
                    None
                },
                value = rx.recv() => Some(value),
            };
            let event = match item {
                Some(Ok(event)) => Event::default().data(
                    serde_json::to_string(&EventView {
                        kind: if event.text.is_some() {
                            "delta"
                        } else {
                            "refresh"
                        }
                        .into(),
                        run_id: event.run_id,
                        text: event.text,
                    })
                    .ok()?,
                ),
                Some(Err(tokio::sync::broadcast::error::RecvError::Closed)) => return None,
                Some(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => Event::default()
                    .data(format!(
                        "{{\"kind\":\"refresh\",\"runId\":\"{run}\",\"text\":null}}"
                    )),
                None => Event::default().comment("keepalive"),
            };
            Some((
                Ok(event),
                (rx, revocations, auth, h, rt, a, next_check, expires_at),
            ))
        },
    );
    Ok(Sse::new(st).keep_alive(KeepAlive::default()))
}

async fn revocation(
    receiver: &mut tokio::sync::broadcast::Receiver<
        factory_application::identity::IdentityRevocation,
    >,
    actor: EnvironmentActor,
) {
    loop {
        match receiver.recv().await {
            Ok(event) if !event.matches(actor.user_id, actor.session_hash) => continue,
            _ => return,
        }
    }
}
