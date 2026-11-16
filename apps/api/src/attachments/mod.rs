//! Upload, removal and reading of conversation attachments. The service owns every
//! rule; this module authenticates, bounds the transfer and applies the response policy.
pub(crate) mod contracts;
mod policy;
mod range;

use self::{contracts::*, range::Selection};
use crate::{
    auth::{AuthError, AuthState},
    contracts::ApiError,
};
use axum::{
    Json,
    body::{Body, BodyDataStream, Bytes},
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{PathRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use factory_application::attachments::AttachmentService;
use factory_core::{
    attachments::{
        Attachment, AttachmentError, AttachmentState, ByteSource, MAX_ATTACHMENT_BYTES,
        MAX_NAME_BYTES, MAX_READ_CHUNK_BYTES,
    },
    environments::EnvironmentActor,
};
use futures_util::{StreamExt, stream};
use std::{io, sync::Arc};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

const OCTET_STREAM: &str = "application/octet-stream";

#[derive(Clone)]
struct StateData {
    auth: Option<AuthState>,
    service: Option<Arc<AttachmentService>>,
}

impl StateData {
    async fn authenticate(
        &self,
        headers: &HeaderMap,
        mutation: bool,
    ) -> Result<(AuthState, EnvironmentActor), Response> {
        let auth = self.auth.clone().ok_or_else(|| {
            AuthError::from(factory_core::identity::IdentityError::Unavailable).into_response()
        })?;
        auth.check_quota(false)
            .map_err(IntoResponse::into_response)?;
        let authenticated = auth
            .authenticate(headers)
            .await
            .map_err(IntoResponse::into_response)?;
        if mutation {
            // The body is raw bytes or empty: no JSON content type is required.
            auth.authorize_mutation(headers, &authenticated, false)
                .map_err(IntoResponse::into_response)?;
        }
        let actor = EnvironmentActor {
            user_id: authenticated.session.user.id,
            session_hash: authenticated.token_hash,
        };
        Ok((auth, actor))
    }

    fn service(&self) -> Result<Arc<AttachmentService>, AttachmentError> {
        self.service.clone().ok_or(AttachmentError::Unavailable)
    }
}

pub(crate) fn documented_router(
    auth: Option<AuthState>,
    service: Option<Arc<AttachmentService>>,
) -> OpenApiRouter {
    let state = StateData { auth, service };
    // Only the upload accepts a large body; the other routes take none.
    let upload = OpenApiRouter::new()
        .routes(routes!(upload))
        .layer(DefaultBodyLimit::max(MAX_ATTACHMENT_BYTES as usize));
    let others = OpenApiRouter::new()
        .routes(routes!(remove))
        .routes(routes!(content))
        .layer(DefaultBodyLimit::max(1024));
    upload.merge(others).with_state(state)
}

pub fn router(auth: AuthState, service: Option<Arc<AttachmentService>>) -> axum::Router {
    documented_router(Some(auth), service).into()
}

fn problem(status: StatusCode, code: &str, message: &str) -> Response {
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

fn error(error: AttachmentError) -> Response {
    let (status, code) = match error {
        AttachmentError::Unauthorized => (StatusCode::UNAUTHORIZED, "session-required"),
        AttachmentError::NotFound => (StatusCode::NOT_FOUND, "not-found"),
        AttachmentError::Conflict => (StatusCode::CONFLICT, "conflict"),
        AttachmentError::Invalid => (StatusCode::BAD_REQUEST, "invalid-input"),
        AttachmentError::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "file-too-large"),
        AttachmentError::TooMany => (StatusCode::CONFLICT, "too-many-attachments"),
        AttachmentError::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
    };
    problem(status, code, &error.to_string())
}

fn invalid() -> Response {
    error(AttachmentError::Invalid)
}

/// The request body as upload chunks, refusing more than the attachment limit even
/// when the client did not declare a length.
struct BodySource {
    stream: BodyDataStream,
    received: u64,
}

#[async_trait::async_trait]
impl ByteSource for BodySource {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, AttachmentError> {
        loop {
            match self.stream.next().await {
                None => return Ok(None),
                Some(Err(_)) => return Err(AttachmentError::Invalid),
                Some(Ok(chunk)) if chunk.is_empty() => {}
                Some(Ok(chunk)) => {
                    self.received += chunk.len() as u64;
                    if self.received > MAX_ATTACHMENT_BYTES {
                        return Err(AttachmentError::TooLarge);
                    }
                    return Ok(Some(chunk.to_vec()));
                }
            }
        }
    }
}

fn declared_length(headers: &HeaderMap) -> Result<Option<u64>, AttachmentError> {
    let mut values = headers.get_all(header::CONTENT_LENGTH).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(AttachmentError::Invalid);
    }
    value
        .to_str()
        .ok()
        .and_then(|text| text.parse().ok())
        .map(Some)
        .ok_or(AttachmentError::Invalid)
}

fn is_octet_stream(headers: &HeaderMap) -> bool {
    let mut values = headers.get_all(header::CONTENT_TYPE).iter();
    match (values.next(), values.next()) {
        (Some(value), None) => value.to_str().is_ok_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|media| media.trim().eq_ignore_ascii_case(OCTET_STREAM))
        }),
        _ => false,
    }
}

#[utoipa::path(post,path="/api/conversations/{id}/attachments",operation_id="uploadConversationAttachment",request_body(content=BinaryBody,content_type="application/octet-stream",description="Raw file bytes, streamed up to 50 MiB"),params(("id"=Uuid,Path),("name"=String,Query,description="File name, a single path component"),("type"=Option<String>,Query,description="Declared type; ignored, Factory classifies the bytes"),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=201,description="The staged attachment",body=AttachmentView),(status=400,description="Invalid path, name or body",body=ApiError),(status=401,description="Active session required",body=ApiError),(status=403,description="Exact Origin or CSRF rejected",body=ApiError),(status=404,body=ApiError),(status=409,description="Too many staged files or a changed conversation",body=ApiError),(status=413,description="Larger than 50 MiB",body=ApiError),(status=415,description="The body is not application/octet-stream",body=ApiError),(status=429,description="Request rate limit",body=ApiError),(status=503,description="Service unavailable",body=ApiError)))]
async fn upload(
    State(state): State<StateData>,
    path: Result<Path<Uuid>, PathRejection>,
    query: Result<Query<UploadQuery>, QueryRejection>,
    headers: HeaderMap,
    body: Body,
) -> Result<(StatusCode, Json<AttachmentView>), Response> {
    let (_, actor) = state.authenticate(&headers, true).await?;
    let Path(conversation) = path.map_err(|_| invalid())?;
    let Query(query) = query.map_err(|_| invalid())?;
    if query
        .declared_type
        .as_ref()
        .is_some_and(|declared| declared.len() > MAX_NAME_BYTES)
    {
        return Err(invalid());
    }
    if !is_octet_stream(&headers) {
        return Err(problem(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported-media-type",
            "Send the file as application/octet-stream.",
        ));
    }
    if declared_length(&headers)
        .map_err(error)?
        .is_some_and(|length| length > MAX_ATTACHMENT_BYTES)
    {
        return Err(error(AttachmentError::TooLarge));
    }
    let service = state.service().map_err(error)?;
    // A client that disconnects ends the body with an error, so the upload always
    // runs to its own cleanup instead of being dropped halfway.
    let attachment = tokio::spawn(async move {
        let mut source = BodySource {
            stream: body.into_data_stream(),
            received: 0,
        };
        service
            .upload(actor, conversation, &query.name, &mut source)
            .await
    })
    .await
    .map_err(|_| error(AttachmentError::Unavailable))?
    .map_err(error)?;
    Ok((StatusCode::CREATED, Json(attachment.into())))
}

#[utoipa::path(delete,path="/api/conversations/{id}/attachments/{attachment}",operation_id="removeConversationAttachment",params(("id"=Uuid,Path),("attachment"=Uuid,Path),("Origin"=String,Header,description="Exact configured frontend Origin required"),("X-CSRF-Token"=String,Header,description="Session CSRF token required")),security(("sessionCookie"=[])),responses((status=204),(status=400,description="Invalid path or body",body=ApiError),(status=401,description="Active session required",body=ApiError),(status=403,description="Exact Origin or CSRF rejected",body=ApiError),(status=404,body=ApiError),(status=409,description="Only staged attachments can be removed",body=ApiError),(status=429,description="Request rate limit",body=ApiError),(status=503,description="Service unavailable",body=ApiError)))]
async fn remove(
    State(state): State<StateData>,
    path: Result<Path<(Uuid, Uuid)>, PathRejection>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, Response> {
    let (_, actor) = state.authenticate(&headers, true).await?;
    let Path((conversation, id)) = path.map_err(|_| invalid())?;
    if !body.is_empty() {
        return Err(invalid());
    }
    let service = state.service().map_err(error)?;
    tokio::spawn(async move {
        let attachment = service.get(actor, id).await?;
        if attachment.conversation_id != conversation {
            return Err(AttachmentError::NotFound);
        }
        service.remove_staged(actor, id).await
    })
    .await
    .map_err(|_| error(AttachmentError::Unavailable))?
    .map_err(error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The bytes of one window of the file, read in bounded chunks so a large file is
/// never held in memory. Every chunk is authorized again; a short read ends the
/// body with an error rather than a silently truncated file.
fn stream_body(
    service: Arc<AttachmentService>,
    actor: EnvironmentActor,
    id: Uuid,
    start: u64,
    length: u64,
    first: Vec<u8>,
) -> Body {
    struct Cursor {
        service: Arc<AttachmentService>,
        actor: EnvironmentActor,
        id: Uuid,
        offset: u64,
        remaining: u64,
        first: Option<Vec<u8>>,
    }
    let cursor = Cursor {
        service,
        actor,
        id,
        offset: start,
        remaining: length,
        first: Some(first),
    };
    Body::from_stream(stream::unfold(cursor, |mut cursor| async move {
        if cursor.remaining == 0 {
            return None;
        }
        let chunk = match cursor.first.take() {
            Some(chunk) => Ok(chunk),
            None => cursor
                .service
                .read(
                    cursor.actor,
                    cursor.id,
                    cursor.offset,
                    cursor.remaining.min(MAX_READ_CHUNK_BYTES),
                )
                .await
                .map(|(_, bytes)| bytes),
        };
        match chunk {
            Ok(bytes) if !bytes.is_empty() && bytes.len() as u64 <= cursor.remaining => {
                cursor.offset += bytes.len() as u64;
                cursor.remaining -= bytes.len() as u64;
                Some((Ok(Bytes::from(bytes)), cursor))
            }
            _ => {
                cursor.remaining = 0;
                Some((Err(io::Error::other("attachment read failed")), cursor))
            }
        }
    }))
}

fn readable(attachment: &Attachment, conversation: Uuid) -> Result<(), AttachmentError> {
    if attachment.conversation_id != conversation
        || !matches!(
            attachment.state,
            AttachmentState::Staged | AttachmentState::Attached
        )
    {
        return Err(AttachmentError::NotFound);
    }
    Ok(())
}

#[utoipa::path(get,path="/api/conversations/{id}/attachments/{attachment}/content",operation_id="readConversationAttachment",params(("id"=Uuid,Path),("attachment"=Uuid,Path),("download"=Option<String>,Query,description="1 forces a download"),("Range"=Option<String>,Header,description="A single byte range")),security(("sessionCookie"=[])),responses((status=200,description="The stored bytes with the type Factory classified",content_type="application/octet-stream",body=BinaryBody),(status=206,description="The requested byte range",content_type="application/octet-stream",body=BinaryBody),(status=400,body=ApiError),(status=401,body=ApiError),(status=404,body=ApiError),(status=416,description="The range is outside the file",body=ApiError),(status=429,body=ApiError),(status=503,body=ApiError)))]
async fn content(
    State(state): State<StateData>,
    path: Result<Path<(Uuid, Uuid)>, PathRejection>,
    query: Result<Query<ContentQuery>, QueryRejection>,
    headers: HeaderMap,
) -> Result<Response, Response> {
    let (auth, actor) = state.authenticate(&headers, false).await?;
    let Path((conversation, id)) = path.map_err(|_| invalid())?;
    let Query(query) = query.map_err(|_| invalid())?;
    let download = match query.download.as_deref() {
        None | Some("0") => false,
        Some("1") => true,
        Some(_) => return Err(invalid()),
    };
    let service = state.service().map_err(error)?;
    let attachment = service.get(actor, id).await.map_err(error)?;
    readable(&attachment, conversation).map_err(error)?;
    let size = attachment.size;
    let requested = if headers.contains_key(header::IF_RANGE) {
        // No validators are issued, so a conditional range can never match.
        None
    } else {
        headers
            .get(header::RANGE)
            .map(|value| value.to_str().map_err(|_| invalid()))
            .transpose()?
    };
    let (status, start, length) = match range::select(requested, size) {
        Selection::Whole => (StatusCode::OK, 0, size),
        Selection::Slice { start, end } => (StatusCode::PARTIAL_CONTENT, start, end - start + 1),
        Selection::Unsatisfiable => {
            let mut response = problem(
                StatusCode::RANGE_NOT_SATISFIABLE,
                "range-not-satisfiable",
                "The requested range is outside the file.",
            );
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                response.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return Ok(response);
        }
    };
    // The first chunk is read before the headers so a failure still has a status.
    let first = if length == 0 {
        Vec::new()
    } else {
        service
            .read(actor, id, start, length.min(MAX_READ_CHUNK_BYTES))
            .await
            .map_err(error)?
            .1
    };
    let body = if length == 0 {
        Body::empty()
    } else {
        stream_body(service, actor, id, start, length, first)
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let response_headers = response.headers_mut();
    response_headers.extend(policy::headers(&attachment, auth.web_origin(), download));
    response_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    if status == StatusCode::PARTIAL_CONTENT
        && let Ok(value) = HeaderValue::from_str(&format!(
            "bytes {start}-{}/{size}",
            start + length.saturating_sub(1)
        ))
    {
        response_headers.insert(header::CONTENT_RANGE, value);
    }
    Ok(response)
}
