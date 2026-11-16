//! Attachment routes against in-memory fakes: no Docker, no PostgreSQL.
use super::*;
use axum::{
    body::Bytes,
    http::{HeaderMap, Method, header},
    response::Response,
};
use factory_application::attachments::AttachmentService;
use factory_core::attachments::{
    AgentAttachment, Attachment, AttachmentError, AttachmentFiles, AttachmentLocation,
    AttachmentSource, AttachmentState, AttachmentStore, ByteSource, MAX_ATTACHMENT_BYTES,
    MAX_READ_CHUNK_BYTES, MediaKind, NewAttachment, StoredFile, WorkerDirectories, classify,
};
use std::collections::BTreeMap;

type Stored = Arc<Mutex<BTreeMap<Uuid, Vec<u8>>>>;

/// Attachment rows held in memory. Ownership follows the fixture's conversation.
struct Rows {
    conversations: Arc<Conversations>,
    rows: Mutex<Vec<Attachment>>,
}

impl Rows {
    fn owned(&self, actor: EnvironmentActor, conversation: Uuid) -> Result<(), AttachmentError> {
        self.conversations
            .owned(actor, conversation)
            .map(|_| ())
            .map_err(|_| AttachmentError::NotFound)
    }
    fn set_state(&self, id: Uuid, state: AttachmentState) {
        for row in self.rows.lock().unwrap().iter_mut().filter(|r| r.id == id) {
            row.state = state;
        }
    }
}

#[async_trait]
impl AttachmentStore for Rows {
    async fn reserve(
        &self,
        actor: EnvironmentActor,
        _: Uuid,
        _: i64,
        new: NewAttachment,
    ) -> Result<Attachment, AttachmentError> {
        self.owned(actor, new.conversation_id)?;
        let row = Attachment {
            id: new.id,
            conversation_id: new.conversation_id,
            run_id: None,
            message_id: None,
            source: AttachmentSource::User,
            state: AttachmentState::Uploading,
            name: new.name,
            media_type: "application/octet-stream".into(),
            kind: MediaKind::File,
            size: 0,
            created_at: Utc::now(),
        };
        self.rows.lock().unwrap().push(row.clone());
        Ok(row)
    }
    async fn mark_staged(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        id: Uuid,
        file: StoredFile,
    ) -> Result<Attachment, AttachmentError> {
        let mut rows = self.rows.lock().unwrap();
        let row = rows
            .iter_mut()
            .find(|r| r.id == id && r.state == AttachmentState::Uploading)
            .ok_or(AttachmentError::Conflict)?;
        row.state = AttachmentState::Staged;
        row.media_type = file.media_type;
        row.kind = file.kind;
        row.size = file.size;
        Ok(row.clone())
    }
    async fn remove_staged(
        &self,
        actor: EnvironmentActor,
        _: Uuid,
        _: i64,
        id: Uuid,
    ) -> Result<Attachment, AttachmentError> {
        let mut rows = self.rows.lock().unwrap();
        let index = rows
            .iter()
            .position(|r| r.id == id)
            .ok_or(AttachmentError::NotFound)?;
        self.owned(actor, rows[index].conversation_id)?;
        if rows[index].state != AttachmentState::Staged {
            return Err(AttachmentError::Conflict);
        }
        Ok(rows.remove(index))
    }
    async fn expire_staged(
        &self,
        _: EnvironmentActor,
        _: DateTime<Utc>,
    ) -> Result<Vec<Attachment>, AttachmentError> {
        Ok(Vec::new())
    }
    async fn get(&self, actor: EnvironmentActor, id: Uuid) -> Result<Attachment, AttachmentError> {
        let row = self
            .rows
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.id == id)
            .cloned()
            .ok_or(AttachmentError::NotFound)?;
        self.owned(actor, row.conversation_id)?;
        Ok(row)
    }
    async fn list(
        &self,
        actor: EnvironmentActor,
        _: Uuid,
        _: i64,
        conversation: Uuid,
    ) -> Result<Vec<Attachment>, AttachmentError> {
        self.owned(actor, conversation)?;
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.conversation_id == conversation)
            .cloned()
            .collect())
    }
    async fn record_agent_attachments(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Uuid,
        _: Uuid,
        _: Vec<AgentAttachment>,
    ) -> Result<Vec<Attachment>, AttachmentError> {
        unreachable!()
    }
}

/// File bytes held in memory, classified like the container controller does.
struct Volume {
    stored: Stored,
    reads: AtomicUsize,
}

#[async_trait]
impl AttachmentFiles for Volume {
    async fn put(
        &self,
        _: &Environment,
        location: &AttachmentLocation,
        source: &mut dyn ByteSource,
    ) -> Result<StoredFile, AttachmentError> {
        let mut bytes = Vec::new();
        while let Some(chunk) = source.next_chunk().await? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
                return Err(AttachmentError::TooLarge);
            }
        }
        let class = classify(
            location.file().rsplit('/').next().unwrap(),
            &bytes[..bytes.len().min(8192)],
        );
        let file = StoredFile {
            media_type: class.media_type.into(),
            kind: class.kind,
            size: bytes.len() as u64,
            sha256: "0".repeat(64),
        };
        self.stored
            .lock()
            .unwrap()
            .insert(location.attachment_id(), bytes);
        Ok(file)
    }
    async fn capture(
        &self,
        _: &Environment,
        _: &AttachmentLocation,
        _: &str,
        _: &[String],
    ) -> Result<StoredFile, AttachmentError> {
        unreachable!()
    }
    async fn read(
        &self,
        _: &Environment,
        location: &AttachmentLocation,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, AttachmentError> {
        assert!(length <= MAX_READ_CHUNK_BYTES);
        self.reads.fetch_add(1, Ordering::SeqCst);
        let stored = self.stored.lock().unwrap();
        let bytes = stored
            .get(&location.attachment_id())
            .ok_or(AttachmentError::NotFound)?;
        let start = (offset as usize).min(bytes.len());
        let end = (start + length as usize).min(bytes.len());
        Ok(bytes[start..end].to_vec())
    }
    async fn delete_attachment(
        &self,
        _: &Environment,
        location: &AttachmentLocation,
    ) -> Result<(), AttachmentError> {
        self.stored
            .lock()
            .unwrap()
            .remove(&location.attachment_id());
        Ok(())
    }
    async fn delete_conversation(
        &self,
        _: &Environment,
        _: Option<Uuid>,
        _: Uuid,
    ) -> Result<(), AttachmentError> {
        Ok(())
    }
    async fn delete_project(&self, _: &Environment, _: Uuid) -> Result<(), AttachmentError> {
        Ok(())
    }
    async fn prepare_worker_directories(
        &self,
        _: &Environment,
        _: Uuid,
        _: Uuid,
        _: Uuid,
    ) -> Result<WorkerDirectories, AttachmentError> {
        unreachable!()
    }
}

struct AttachmentFixture {
    base: ConversationFixture,
    app: Router,
    rows: Arc<Rows>,
    files: Arc<Volume>,
    conversation: Uuid,
}

impl AttachmentFixture {
    async fn new() -> Self {
        let base = ConversationFixture::new().await;
        let created = base.call("POST", "/api/conversations", 0, "").await;
        assert_eq!(created.status(), StatusCode::OK);
        let conversation = body_json(created).await["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let rows = Arc::new(Rows {
            conversations: base.data.clone(),
            rows: Mutex::default(),
        });
        let files = Arc::new(Volume {
            stored: Arc::default(),
            reads: AtomicUsize::new(0),
        });
        let service = Arc::new(AttachmentService::new(
            base.conversation_service.clone(),
            rows.clone(),
            files.clone(),
        ));
        let app = factory_api::attachments::router(base.auth.clone(), Some(service));
        Self {
            base,
            app,
            rows,
            files,
            conversation,
        }
    }

    fn url(&self) -> String {
        format!("/api/conversations/{}/attachments", self.conversation)
    }

    fn request(&self, method: Method, uri: &str, user: usize) -> axum::http::request::Builder {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(
                header::COOKIE,
                format!("factory_session={}", self.base.tokens[user]),
            )
            .header(header::ORIGIN, ORIGIN)
            .header("x-csrf-token", CSRF)
    }

    async fn send(&self, request: Request<Body>) -> Response {
        self.app.clone().oneshot(request).await.unwrap()
    }

    async fn upload_as(&self, user: usize, name: &str, body: Vec<u8>) -> Response {
        let request = self
            .request(Method::POST, &format!("{}?name={name}", self.url()), user)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(body))
            .unwrap();
        self.send(request).await
    }

    /// Upload as the owner and return the attachment ID.
    async fn upload(&self, name: &str, body: Vec<u8>) -> Uuid {
        let response = self.upload_as(0, name, body).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        body_json(response).await["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    }

    async fn read(&self, id: Uuid, query: &str, range: Option<&str>) -> Response {
        let mut request = self.request(
            Method::GET,
            &format!("{}/{id}/content{query}", self.url()),
            0,
        );
        if let Some(range) = range {
            request = request.header(header::RANGE, range);
        }
        self.send(request.body(Body::empty()).unwrap()).await
    }
}

fn header_of(response: &Response, name: impl header::AsHeaderName) -> String {
    response.headers()[name].to_str().unwrap().to_owned()
}

async fn bytes(response: Response) -> Bytes {
    to_bytes(response.into_body(), 64 * 1024 * 1024)
        .await
        .unwrap()
}

fn png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
    bytes
}

/// Bytes with controls and invalid UTF-8, so they classify as an opaque file.
fn pattern(length: usize) -> Vec<u8> {
    (0..length).map(|i| (i % 251) as u8 | 0x80).collect()
}

#[tokio::test]
async fn upload_stages_the_file_and_classifies_the_bytes_not_the_declared_type() {
    let f = AttachmentFixture::new().await;
    let request = f
        .request(
            Method::POST,
            &format!("{}?name=photo.txt&type=text/html", f.url()),
            0,
        )
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(png()))
        .unwrap();
    let response = f.send(request).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let view = body_json(response).await;
    assert_eq!(view["state"], "staged");
    assert_eq!(view["source"], "user");
    assert_eq!(view["name"], "photo.txt");
    assert_eq!(view["mediaType"], "image/png");
    assert_eq!(view["kind"], "image");
    assert_eq!(view["size"], png().len());
    assert_eq!(view["conversationId"], f.conversation.to_string());
    assert!(view["messageId"].is_null() && view["runId"].is_null());
    let content = f
        .read(view["id"].as_str().unwrap().parse().unwrap(), "", None)
        .await;
    assert_eq!(header_of(&content, header::CONTENT_TYPE), "image/png");
}

#[tokio::test]
async fn upload_requires_session_origin_csrf_and_a_binary_body_but_not_json() {
    let f = AttachmentFixture::new().await;
    let uri = format!("{}?name=a.bin", f.url());
    let send = |builder: axum::http::request::Builder| {
        let request = builder
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(pattern(10)))
            .unwrap();
        f.send(request)
    };
    let anonymous = Request::builder().method(Method::POST).uri(&uri);
    assert_eq!(send(anonymous).await.status(), StatusCode::UNAUTHORIZED);
    let cookie = format!("factory_session={}", f.base.tokens[0]);
    let no_origin = Request::builder()
        .method(Method::POST)
        .uri(&uri)
        .header(header::COOKIE, &cookie)
        .header("x-csrf-token", CSRF);
    assert_eq!(send(no_origin).await.status(), StatusCode::FORBIDDEN);
    let wrong_origin = f
        .request(Method::POST, &uri, 0)
        .header(header::ORIGIN, "https://evil.example");
    assert_eq!(send(wrong_origin).await.status(), StatusCode::FORBIDDEN);
    let no_csrf = Request::builder()
        .method(Method::POST)
        .uri(&uri)
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, ORIGIN);
    assert_eq!(send(no_csrf).await.status(), StatusCode::FORBIDDEN);
    let bad_csrf = f
        .request(Method::POST, &uri, 0)
        .header("x-csrf-token", "forged");
    assert_eq!(send(bad_csrf).await.status(), StatusCode::FORBIDDEN);

    for content_type in ["application/json", "text/plain"] {
        let request = f
            .request(Method::POST, &uri, 0)
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(pattern(10)))
            .unwrap();
        assert_eq!(
            f.send(request).await.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
    }
    let missing = f
        .request(Method::POST, &uri, 0)
        .body(Body::from(pattern(10)))
        .unwrap();
    assert_eq!(
        f.send(missing).await.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert!(f.rows.rows.lock().unwrap().is_empty());
    assert_eq!(f.upload_as(0, "a.bin", pattern(10)).await.status(), 201);
}

#[tokio::test]
async fn attachments_are_private_to_the_owner_and_the_conversation_in_the_path() {
    let f = AttachmentFixture::new().await;
    let id = f.upload("secret.bin", pattern(64)).await;
    assert_eq!(
        f.upload_as(1, "x.bin", pattern(4)).await.status(),
        StatusCode::NOT_FOUND
    );
    let foreign = |method: Method, suffix: &str| {
        f.request(method, &format!("{}/{id}{suffix}", f.url()), 1)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        f.send(foreign(Method::GET, "/content")).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.send(foreign(Method::DELETE, "")).await.status(),
        StatusCode::NOT_FOUND
    );
    // The owner still cannot reach the file through another conversation.
    let other = Uuid::new_v4();
    let wrong_conversation = |method: Method, suffix: &str| {
        f.request(
            method,
            &format!("/api/conversations/{other}/attachments/{id}{suffix}"),
            0,
        )
        .body(Body::empty())
        .unwrap()
    };
    assert_eq!(
        f.send(wrong_conversation(Method::GET, "/content"))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.send(wrong_conversation(Method::DELETE, ""))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.upload_as(0, "late.bin", pattern(4)).await.status(),
        StatusCode::CREATED
    );
    assert_eq!(f.read(id, "", None).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn upload_enforces_the_size_and_count_limits() {
    let f = AttachmentFixture::new().await;
    let uri = format!("{}?name=big.bin", f.url());
    // A declared length over the limit is refused before the body is read.
    let declared = f
        .request(Method::POST, &uri, 0)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, MAX_ATTACHMENT_BYTES + 1)
        .body(Body::empty())
        .unwrap();
    let response = f.send(declared).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body_json(response).await["code"], "file-too-large");

    // A stream without a declared length is cut off at the limit and leaves no file.
    let chunk = Bytes::from(vec![7u8; 1024 * 1024]);
    let chunks = MAX_ATTACHMENT_BYTES / chunk.len() as u64 + 1;
    let endless = futures_util::stream::iter(
        (0..chunks).map(move |_| Ok::<_, std::io::Error>(chunk.clone())),
    );
    let streamed = f
        .request(Method::POST, &uri, 0)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from_stream(endless))
        .unwrap();
    assert_eq!(
        f.send(streamed).await.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert!(f.files.stored.lock().unwrap().is_empty());

    // Exactly the limit is accepted.
    let exact = f
        .upload_as(0, "exact.bin", pattern(MAX_ATTACHMENT_BYTES as usize))
        .await;
    assert_eq!(exact.status(), StatusCode::CREATED);
    assert_eq!(body_json(exact).await["size"], MAX_ATTACHMENT_BYTES);
    f.files.stored.lock().unwrap().clear();
    f.rows.rows.lock().unwrap().clear();

    for index in 0..10 {
        f.upload(&format!("file-{index}.bin"), pattern(8)).await;
    }
    let eleventh = f.upload_as(0, "overflow.bin", pattern(8)).await;
    assert_eq!(eleventh.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(eleventh).await["code"], "too-many-attachments");
}

#[tokio::test]
async fn upload_names_must_be_a_single_path_component() {
    let f = AttachmentFixture::new().await;
    for name in ["..", ".", "a%2Fb", "a%00b", "%2E%2E%2Fescape", ""] {
        assert_eq!(
            f.upload_as(0, name, pattern(4)).await.status(),
            StatusCode::BAD_REQUEST,
            "{name:?}"
        );
    }
    let missing = f
        .request(Method::POST, &f.url(), 0)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(pattern(4)))
        .unwrap();
    assert_eq!(f.send(missing).await.status(), StatusCode::BAD_REQUEST);
    let unknown = f
        .request(Method::POST, &format!("{}?name=a&other=1", f.url()), 0)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(pattern(4)))
        .unwrap();
    assert_eq!(f.send(unknown).await.status(), StatusCode::BAD_REQUEST);
    assert!(f.rows.rows.lock().unwrap().is_empty());
    assert_eq!(
        f.upload_as(0, "caf%C3%A9.txt", b"text".to_vec())
            .await
            .status(),
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn only_staged_attachments_can_be_removed() {
    let f = AttachmentFixture::new().await;
    let id = f.upload("draft.bin", pattern(32)).await;
    let delete = |id: Uuid, user: usize, csrf: Option<&str>, body: &'static str| {
        let mut request = f.request(Method::DELETE, &format!("{}/{id}", f.url()), user);
        if let Some(csrf) = csrf {
            request = request.header("x-csrf-token", csrf);
        }
        request.body(Body::from(body)).unwrap()
    };
    assert_eq!(
        f.send(delete(id, 0, Some("forged"), "")).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.send(delete(id, 0, None, "{}")).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.send(delete(id, 0, None, "")).await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(f.files.stored.lock().unwrap().is_empty());
    assert_eq!(f.read(id, "", None).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        f.send(delete(id, 0, None, "")).await.status(),
        StatusCode::NOT_FOUND
    );

    let sent = f.upload("sent.bin", pattern(32)).await;
    f.rows.set_state(sent, AttachmentState::Attached);
    let refused = f.send(delete(sent, 0, None, "")).await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(f.read(sent, "", None).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn content_serves_one_range_and_streams_across_chunks() {
    let f = AttachmentFixture::new().await;
    let length = 2 * MAX_READ_CHUNK_BYTES as usize + 1000;
    let data = pattern(length);
    let id = f.upload("movie.bin", data.clone()).await;

    let whole = f.read(id, "", None).await;
    assert_eq!(whole.status(), StatusCode::OK);
    assert_eq!(
        header_of(&whole, header::CONTENT_LENGTH),
        length.to_string()
    );
    assert_eq!(header_of(&whole, header::ACCEPT_RANGES), "bytes");
    assert!(whole.headers().get(header::CONTENT_RANGE).is_none());
    let reads = f.files.reads.load(Ordering::SeqCst);
    assert_eq!(bytes(whole).await.as_ref(), data.as_slice());
    // The first chunk was read for the headers, then the body followed in chunks.
    assert_eq!(f.files.reads.load(Ordering::SeqCst) - reads, 2);

    let edge = MAX_READ_CHUNK_BYTES as usize;
    let cases: [(&str, usize, usize); 5] = [
        ("bytes=0-0", 0, 1),
        ("bytes=10-19", 10, 10),
        ("bytes=-100", length - 100, 100),
        ("bytes=4194300-4194310", edge - 4, 11),
        ("bytes=4194304-99999999999", edge, length - edge),
    ];
    for (range, start, count) in cases {
        let part = f.read(id, "", Some(range)).await;
        assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT, "{range}");
        assert_eq!(header_of(&part, header::CONTENT_LENGTH), count.to_string());
        assert_eq!(
            header_of(&part, header::CONTENT_RANGE),
            format!("bytes {start}-{}/{length}", start + count - 1)
        );
        assert_eq!(
            bytes(part).await.as_ref(),
            &data[start..start + count],
            "{range}"
        );
    }

    for range in [
        format!("bytes={length}-"),
        "bytes=5-2".into(),
        "bytes=0-1,5-6".into(),
        "bytes=-0".into(),
        "bytes=x-y".into(),
    ] {
        let refused = f.read(id, "", Some(&range)).await;
        assert_eq!(
            refused.status(),
            StatusCode::RANGE_NOT_SATISFIABLE,
            "{range}"
        );
        assert_eq!(
            header_of(&refused, header::CONTENT_RANGE),
            format!("bytes */{length}")
        );
        assert_eq!(body_json(refused).await["code"], "range-not-satisfiable");
    }
    // Other units and conditional ranges fall back to the whole file.
    assert_eq!(
        f.read(id, "", Some("items=0-1")).await.status(),
        StatusCode::OK
    );
    let conditional = f
        .request(Method::GET, &format!("{}/{id}/content", f.url()), 0)
        .header(header::RANGE, "bytes=0-1")
        .header(header::IF_RANGE, "\"unknown\"")
        .body(Body::empty())
        .unwrap();
    assert_eq!(f.send(conditional).await.status(), StatusCode::OK);

    let empty = f.upload("empty.txt", Vec::new()).await;
    let response = f.read(empty, "", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header_of(&response, header::CONTENT_LENGTH), "0");
    assert!(bytes(response).await.is_empty());
    assert_eq!(
        f.read(empty, "", Some("bytes=0-")).await.status(),
        StatusCode::RANGE_NOT_SATISFIABLE
    );
}

#[tokio::test]
async fn content_is_not_available_before_or_without_bytes() {
    let f = AttachmentFixture::new().await;
    let id = f.upload("note.txt", b"hello".to_vec()).await;
    for state in [AttachmentState::Uploading, AttachmentState::Unavailable] {
        f.rows.set_state(id, state);
        assert_eq!(f.read(id, "", None).await.status(), StatusCode::NOT_FOUND);
    }
    assert_eq!(
        f.read(Uuid::new_v4(), "", None).await.status(),
        StatusCode::NOT_FOUND
    );
    f.rows.set_state(id, AttachmentState::Attached);
    for query in ["?download=2", "?download=", "?other=1"] {
        assert_eq!(
            f.read(id, query, None).await.status(),
            StatusCode::BAD_REQUEST,
            "{query}"
        );
    }
    let anonymous = Request::builder()
        .uri(format!("{}/{id}/content", f.url()))
        .body(Body::empty())
        .unwrap();
    assert_eq!(f.send(anonymous).await.status(), StatusCode::UNAUTHORIZED);
}

const SANDBOX: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

fn expect_policy(response: &Response, content_type: &str, disposition: &str, policy: &str) {
    let headers: &HeaderMap = response.headers();
    assert_eq!(headers[header::CONTENT_TYPE], content_type);
    assert!(
        headers[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with(disposition)
    );
    assert_eq!(headers[header::CONTENT_SECURITY_POLICY], policy);
    assert_eq!(headers["cross-origin-resource-policy"], "same-site");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn response_headers_follow_the_stored_kind() {
    let f = AttachmentFixture::new().await;
    let mut mp4 = vec![0, 0, 0, 0x18];
    mp4.extend_from_slice(b"ftypisom\0\0\0\0isomiso2");
    let cases: Vec<(&str, Vec<u8>, &str, &str, String)> = vec![
        ("pic.png", png(), "image/png", "inline", SANDBOX.into()),
        (
            "logo.svg",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>".to_vec(),
            "image/svg+xml",
            "inline",
            SANDBOX.into(),
        ),
        ("clip.mp4", mp4, "video/mp4", "inline", SANDBOX.into()),
        (
            "doc.pdf",
            b"%PDF-1.7\n".to_vec(),
            "application/pdf",
            "inline",
            format!("frame-ancestors {ORIGIN}"),
        ),
        (
            "page.html",
            b"<!doctype html><script>alert(1)</script>".to_vec(),
            "text/plain; charset=utf-8",
            "inline",
            SANDBOX.into(),
        ),
        (
            "notes.md",
            b"# notes".to_vec(),
            "text/plain; charset=utf-8",
            "inline",
            SANDBOX.into(),
        ),
        (
            "data.bin",
            pattern(64),
            "application/octet-stream",
            "attachment",
            SANDBOX.into(),
        ),
    ];
    for (name, content, content_type, disposition, policy) in cases {
        let id = f.upload(name, content).await;
        let response = f.read(id, "", None).await;
        assert_eq!(response.status(), StatusCode::OK, "{name}");
        expect_policy(&response, content_type, disposition, &policy);
        let download = f.read(id, "?download=1", None).await;
        assert!(
            header_of(&download, header::CONTENT_DISPOSITION).starts_with("attachment;"),
            "{name}"
        );
        // Forced downloads keep the same containment.
        assert_eq!(
            header_of(&download, header::CONTENT_SECURITY_POLICY),
            policy,
            "{name}"
        );
        f.files.stored.lock().unwrap().clear();
        f.rows.rows.lock().unwrap().clear();
    }
    // PDF viewers cannot run inside a CSP sandbox.
    let pdf = f.upload("doc.pdf", b"%PDF-1.4".to_vec()).await;
    let policy = header_of(
        &f.read(pdf, "", None).await,
        header::CONTENT_SECURITY_POLICY,
    );
    assert!(!policy.contains("sandbox"));
}

#[tokio::test]
async fn file_names_are_sent_as_rfc_5987_values() {
    let f = AttachmentFixture::new().await;
    let id = f
        .upload("r%C3%A9sum%C3%A9%20%22v2%22.bin", pattern(16))
        .await;
    let response = f.read(id, "", None).await;
    let disposition = header_of(&response, header::CONTENT_DISPOSITION);
    assert_eq!(
        disposition,
        "attachment; filename=\"r_sum_ _v2_.bin\"; filename*=UTF-8''r%C3%A9sum%C3%A9%20%22v2%22.bin"
    );
}

#[tokio::test]
async fn history_groups_attachments_under_their_messages() {
    let f = AttachmentFixture::new().await;
    let run = Uuid::new_v4();
    let (user_message, agent_message) = (Uuid::new_v4(), Uuid::new_v4());
    let attachment = |message: Uuid, source, state, name: &str| Attachment {
        id: Uuid::new_v4(),
        conversation_id: f.conversation,
        run_id: Some(run),
        message_id: Some(message),
        source,
        state,
        name: name.into(),
        media_type: "application/octet-stream".into(),
        kind: MediaKind::File,
        size: 0,
        created_at: Utc::now(),
    };
    {
        let mut data = f.base.data.0.lock().unwrap();
        data.messages = [
            (user_message, MessageRole::User),
            (agent_message, MessageRole::Assistant),
        ]
        .into_iter()
        .map(|(id, role)| Message {
            id,
            run_id: run,
            role,
            content: "text".into(),
            created_at: Utc::now(),
        })
        .collect();
        data.attachments = vec![
            attachment(
                user_message,
                AttachmentSource::User,
                AttachmentState::Attached,
                "in.txt",
            ),
            attachment(
                agent_message,
                AttachmentSource::Agent,
                AttachmentState::Attached,
                "out.png",
            ),
            attachment(
                agent_message,
                AttachmentSource::Agent,
                AttachmentState::Unavailable,
                "gone.pdf",
            ),
        ];
    }
    let response = f
        .base
        .call(
            "GET",
            &format!("/api/conversations/{}", f.conversation),
            0,
            "",
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let history = body_json(response).await;
    let names = |index: usize| -> Vec<(String, String, String)> {
        history["messages"][index]["attachments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                (
                    a["name"].as_str().unwrap().into(),
                    a["source"].as_str().unwrap().into(),
                    a["state"].as_str().unwrap().into(),
                )
            })
            .collect()
    };
    assert_eq!(
        names(0),
        [("in.txt".into(), "user".into(), "attached".into())]
    );
    assert_eq!(
        names(1),
        [
            ("out.png".into(), "agent".into(), "attached".into()),
            ("gone.pdf".into(), "agent".into(), "unavailable".into())
        ]
    );
}

#[tokio::test]
async fn turns_accept_attachment_ids_and_reject_invalid_lists() {
    let f = AttachmentFixture::new().await;
    let path = format!("/api/conversations/{}/runs", f.conversation);
    let body = |ids: Vec<Uuid>| {
        json!({"operationId":Uuid::new_v4(),"model":"fixture-model","input":"look","attachmentIds":ids})
            .to_string()
    };
    let duplicate = Uuid::new_v4();
    assert_eq!(
        f.base
            .call("POST", &path, 0, &body(vec![duplicate, duplicate]))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        f.base
            .call(
                "POST",
                &path,
                0,
                &body((0..11).map(|_| Uuid::new_v4()).collect())
            )
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    // This runtime was composed without attachment support, so files cannot be sent.
    assert_eq!(
        f.base
            .call("POST", &path, 0, &body(vec![Uuid::new_v4()]))
            .await
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    let invalid = json!({"operationId":Uuid::new_v4(),"model":"fixture-model","input":"x","attachmentIds":["nope"]});
    assert_eq!(
        f.base
            .call("POST", &path, 0, &invalid.to_string())
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    // Omitting the list stays valid.
    let plain = json!({"operationId":Uuid::new_v4(),"model":"fixture-model","input":"hi"});
    assert_eq!(
        f.base
            .call("POST", &path, 0, &plain.to_string())
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn unconfigured_services_fail_closed() {
    let f = AttachmentFixture::new().await;
    let app = factory_api::attachments::router(f.base.auth.clone(), None);
    let request = f
        .request(Method::POST, &format!("{}?name=a.bin", f.url()), 0)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(pattern(4)))
        .unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}
