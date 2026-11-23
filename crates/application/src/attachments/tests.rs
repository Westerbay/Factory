use super::{agent::derived_id, *};
use crate::environments::{EnvironmentAvailability, EnvironmentService};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use factory_core::{
    conversations::{Conversation, ConversationHistory, ConversationPage, Message, MessageRole},
    conversations::{ConversationStore, StartTurn},
    conversations::{PendingRequest, RecoverableRun, ResponseClaim, ResponseState, Run, RunStatus},
    environments::*,
    identity::CredentialHash,
};
use std::{collections::VecDeque, future::pending, sync::Mutex};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest-of-the-image";

struct Env(Environment);
#[async_trait]
impl EnvironmentStore for Env {
    async fn get(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        if actor.user_id != self.0.user_id {
            return Err(EnvironmentError::Unauthorized);
        }
        Ok(Some(self.0.clone()))
    }
    async fn initialize_if_absent(
        &self,
        _: EnvironmentActor,
        _: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError> {
        unreachable!()
    }
    async fn start(
        &self,
        _: EnvironmentActor,
        _: EnvironmentStart,
    ) -> Result<Environment, EnvironmentError> {
        unreachable!()
    }
    async fn stop(&self, _: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        unreachable!()
    }
    async fn reconcile_batch(&self, _: Uuid, _: u32) -> Result<Vec<Environment>, EnvironmentError> {
        unreachable!()
    }
    async fn begin_provisioning(&self, _: Uuid, _: i64) -> Result<bool, EnvironmentError> {
        unreachable!()
    }
    async fn record_observation(
        &self,
        _: Uuid,
        _: i64,
        _: EnvironmentObservation,
        _: u32,
    ) -> Result<bool, EnvironmentError> {
        unreachable!()
    }
}

/// Only what attachments use: owned lookup and appending messages.
struct Conversations {
    conversation: Conversation,
    messages: Mutex<Vec<(Uuid, String)>>,
    fail_append: Mutex<bool>,
}
#[rustfmt::skip]
#[async_trait]
impl ConversationStore for Conversations {
    async fn get(&self, actor: EnvironmentActor, _: Uuid, _: i64, id: Uuid) -> Result<Conversation, ConversationError> {
        if actor.user_id == self.conversation.user_id && id == self.conversation.id {
            Ok(self.conversation.clone())
        } else {
            Err(ConversationError::NotFound)
        }
    }
    async fn append_assistant(&self, _: EnvironmentActor, _: Uuid, _: i64, run_id: Uuid, item: &str, text: &str) -> Result<(Message, bool), ConversationError> {
        if *self.fail_append.lock().unwrap() { return Err(ConversationError::Unavailable); }
        if text.is_empty() { return Err(ConversationError::Invalid); }
        let mut messages = self.messages.lock().unwrap();
        let existing = messages.iter().find(|(_, saved)| saved.starts_with(&format!("{item}:")));
        let id = existing.map_or_else(Uuid::new_v4, |(id, _)| *id);
        let inserted = existing.is_none();
        let message = Message { id, run_id, role: MessageRole::Assistant, content: text.to_owned(), created_at: Utc::now() };
        if inserted { messages.push((message.id, format!("{item}:{text}"))); }
        Ok((message, inserted))
    }
    async fn reserve_model_probe(&self, _: EnvironmentActor, _: Uuid, _: i64, _: Uuid) -> Result<(), ConversationError> { unreachable!() }
    async fn settle_model_probe_launch(&self, _: Uuid, _: Uuid, _: i64) -> Result<(), ConversationError> { unreachable!() }
    async fn release_model_probe(&self, _: EnvironmentActor, _: Uuid) -> Result<(), ConversationError> { unreachable!() }
    async fn recover_model_probe_stopped(&self, _: EnvironmentActor, _: Uuid, _: i64, _: chrono::DateTime<Utc>) -> Result<bool, ConversationError> { unreachable!() }
    async fn has_open_runs(&self, _: EnvironmentActor) -> Result<bool, ConversationError> { unreachable!() }
    async fn create(&self, _: EnvironmentActor, _: Uuid, _: i64) -> Result<Conversation, ConversationError> { unreachable!() }
    async fn list(&self, _: EnvironmentActor, _: Uuid, _: i64, _: Option<Uuid>) -> Result<ConversationPage, ConversationError> { unreachable!() }
    async fn history(&self, _: EnvironmentActor, _: Uuid, _: i64, _: Uuid) -> Result<ConversationHistory, ConversationError> { unreachable!() }
    async fn find_turn(&self, _: EnvironmentActor, _: StartTurn) -> Result<Option<Run>, ConversationError> { unreachable!() }
    async fn begin_turn(&self, _: EnvironmentActor, _: StartTurn) -> Result<(Run, bool), ConversationError> { unreachable!() }
    async fn settle_run_launch(&self, _: Uuid, _: Uuid, _: i64) -> Result<(), ConversationError> { unreachable!() }
    async fn set_thread(&self, _: EnvironmentActor, _: Uuid, _: i64, _: Uuid, _: &str, _: Option<&str>) -> Result<(), ConversationError> { unreachable!() }
    async fn add_request(&self, _: EnvironmentActor, _: Uuid, _: i64, _: PendingRequest, _: &str) -> Result<(), ConversationError> { unreachable!() }
    async fn claim_response(&self, _: EnvironmentActor, _: Uuid, _: i64, _: Uuid, _: Uuid, _: &str) -> Result<ResponseClaim, ConversationError> { unreachable!() }
    async fn record_response(&self, _: EnvironmentActor, _: Uuid, _: i64, _: Uuid, _: Uuid, _: ResponseState) -> Result<(), ConversationError> { unreachable!() }
    async fn recoverable_stopped(&self, _: EnvironmentActor, _: Uuid, _: i64, _: chrono::DateTime<Utc>) -> Result<Vec<RecoverableRun>, ConversationError> { unreachable!() }
    async fn recover_stopped(&self, _: EnvironmentActor, _: Uuid, _: i64, _: chrono::DateTime<Utc>, _: Uuid) -> Result<(), ConversationError> { unreachable!() }
    async fn finish_turn(&self, _: EnvironmentActor, _: Uuid, _: i64, _: Uuid, _: RunStatus) -> Result<(), ConversationError> { unreachable!() }
}

#[derive(Default)]
struct Rows {
    rows: Mutex<Vec<Attachment>>,
    conversation: Uuid,
    fail_record: Mutex<bool>,
    fail_after_commit: Mutex<bool>,
}
impl Rows {
    fn all(&self) -> Vec<Attachment> {
        self.rows.lock().unwrap().clone()
    }
    fn change(
        &self,
        id: Uuid,
        change: impl FnOnce(&mut Attachment),
    ) -> Result<Attachment, AttachmentError> {
        let mut rows = self.rows.lock().unwrap();
        let row = rows
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(AttachmentError::NotFound)?;
        change(row);
        Ok(row.clone())
    }
}
#[rustfmt::skip]
#[async_trait]
impl AttachmentStore for Rows {
    async fn reserve(&self, _: EnvironmentActor, _: Uuid, _: i64, new: NewAttachment) -> Result<Attachment, AttachmentError> {
        let row = Attachment { id: new.id, conversation_id: new.conversation_id, run_id: None, message_id: None, source: AttachmentSource::User, state: AttachmentState::Uploading, name: new.name, media_type: OCTET_STREAM.into(), kind: MediaKind::File, size: 0, created_at: Utc::now() };
        self.rows.lock().unwrap().push(row.clone());
        Ok(row)
    }
    async fn mark_staged(&self, _: EnvironmentActor, _: Uuid, _: i64, id: Uuid, file: StoredFile) -> Result<Attachment, AttachmentError> {
        self.change(id, |a| (a.state, a.media_type, a.kind, a.size) = (AttachmentState::Staged, file.media_type, file.kind, file.size))
    }
    async fn remove_staged(&self, _: EnvironmentActor, _: Uuid, _: i64, id: Uuid) -> Result<Attachment, AttachmentError> {
        let mut rows = self.rows.lock().unwrap();
        let index = rows.iter().position(|a| a.id == id).ok_or(AttachmentError::NotFound)?;
        if rows[index].state != AttachmentState::Staged {
            return Err(AttachmentError::Conflict);
        }
        Ok(rows.remove(index))
    }
    async fn expire_staged(&self, _: EnvironmentActor, before: chrono::DateTime<Utc>) -> Result<Vec<Attachment>, AttachmentError> {
        let mut rows = self.rows.lock().unwrap();
        let (old, kept) = rows.drain(..).partition(|a| a.created_at < before && matches!(a.state, AttachmentState::Staged | AttachmentState::Uploading));
        *rows = kept;
        Ok(old)
    }
    async fn get(&self, _: EnvironmentActor, id: Uuid) -> Result<Attachment, AttachmentError> {
        self.all().into_iter().find(|a| a.id == id).ok_or(AttachmentError::NotFound)
    }
    async fn list(&self, _: EnvironmentActor, _: Uuid, _: i64, conversation: Uuid) -> Result<Vec<Attachment>, AttachmentError> {
        Ok(self.all().into_iter().filter(|a| a.conversation_id == conversation).collect())
    }
    async fn record_agent_attachments(&self, _: EnvironmentActor, _: Uuid, _: i64, run_id: Uuid, message_id: Uuid, files: Vec<AgentAttachment>) -> Result<Vec<Attachment>, AttachmentError> {
        if *self.fail_record.lock().unwrap() { return Err(AttachmentError::Unavailable); }
        let mut rows = self.rows.lock().unwrap();
        let existing: Vec<_> = rows.iter().filter(|row| row.message_id == Some(message_id)).cloned().collect();
        if !existing.is_empty() {
            return if existing.len() == files.len() && existing.iter().zip(&files).all(|(a,b)| a.id == b.id && a.name == b.name) { Ok(existing) } else { Err(AttachmentError::Conflict) };
        }
        let mut recorded = Vec::new();
        for file in files {
            let (state, stored) = match file.capture {
                AgentCapture::Captured(stored) => (AttachmentState::Attached, Some(stored)),
                AgentCapture::Unavailable => (AttachmentState::Unavailable, None),
            };
            let row = Attachment {
                id: file.id, conversation_id: self.conversation, run_id: Some(run_id), message_id: Some(message_id), source: AttachmentSource::Agent, state, name: file.name,
                media_type: stored.as_ref().map_or(OCTET_STREAM.into(), |s| s.media_type.clone()), kind: stored.as_ref().map_or(MediaKind::File, |s| s.kind), size: stored.map_or(0, |s| s.size), created_at: Utc::now(),
            };
            rows.push(row.clone());
            recorded.push(row);
        }
        if *self.fail_after_commit.lock().unwrap() { Err(AttachmentError::Unavailable) } else { Ok(recorded) }
    }
}

#[derive(Default)]
struct Files {
    stored: Mutex<HashMap<String, Vec<u8>>>,
    /// Container files the agent can reference, by absolute path.
    sources: Mutex<HashMap<String, Vec<u8>>>,
    captures: Mutex<Vec<(String, Vec<String>)>>,
    stall: Mutex<bool>,
    copy_started: tokio::sync::Notify,
    copy_release: tokio::sync::Notify,
    capture_error: Mutex<Option<AttachmentError>>,
    delete_error: Mutex<Option<AttachmentError>>,
    max_put: Mutex<Option<usize>>,
    purged_projects: Mutex<Vec<Uuid>>,
}
impl Files {
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.stored.lock().unwrap().keys().cloned().collect();
        names.sort();
        names
    }
    fn stored(file: &[u8], name: &str) -> StoredFile {
        let class = classify(name, file);
        StoredFile {
            media_type: class.media_type.into(),
            kind: class.kind,
            size: file.len() as u64,
            sha256: "0".repeat(64),
        }
    }
}
#[rustfmt::skip]
#[async_trait]
impl AttachmentFiles for Files {
    async fn put(&self, _: &Environment, location: &AttachmentLocation, source: &mut dyn ByteSource) -> Result<StoredFile, AttachmentError> {
        let mut bytes = Vec::new();
        while let Some(chunk) = source.next_chunk().await? {
            bytes.extend(chunk);
            if self.max_put.lock().unwrap().is_some_and(|max| bytes.len() > max) {
                return Err(AttachmentError::TooLarge);
            }
        }
        let stored = Self::stored(&bytes, &location.file());
        self.stored.lock().unwrap().insert(location.file(), bytes);
        Ok(stored)
    }
    async fn capture(&self, _: &Environment, location: &AttachmentLocation, reference: &str, roots: &[String]) -> Result<StoredFile, AttachmentError> {
        self.captures.lock().unwrap().push((reference.to_owned(), roots.to_vec()));
        if let Some(error) = *self.capture_error.lock().unwrap() { return Err(error); }
        if *self.stall.lock().unwrap() {
            self.copy_started.notify_one();
            self.copy_release.notified().await;
        }
        if let Some(bytes) = self.stored.lock().unwrap().get(&location.file()) {
            return Ok(Self::stored(bytes, &location.file()));
        }
        let bytes = self.sources.lock().unwrap().get(reference).cloned().ok_or(AttachmentError::NotFound)?;
        let stored = Self::stored(&bytes, &location.file());
        self.stored.lock().unwrap().insert(location.file(), bytes);
        Ok(stored)
    }
    async fn read(&self, _: &Environment, location: &AttachmentLocation, offset: u64, length: u64) -> Result<Vec<u8>, AttachmentError> {
        let stored = self.stored.lock().unwrap();
        let bytes = stored.get(&location.file()).ok_or(AttachmentError::NotFound)?;
        Ok(bytes.iter().skip(offset as usize).take(length as usize).copied().collect())
    }
    async fn delete_attachment(&self, _: &Environment, location: &AttachmentLocation) -> Result<(), AttachmentError> {
        if let Some(error) = *self.delete_error.lock().unwrap() { return Err(error); }
        self.stored.lock().unwrap().remove(&location.file());
        Ok(())
    }
    async fn delete_conversation(&self, _: &Environment, _: Option<Uuid>, _: Uuid) -> Result<(), AttachmentError> { unreachable!() }
    async fn delete_project(&self, _: &Environment, project_id: Uuid) -> Result<(), AttachmentError> {
        self.purged_projects.lock().unwrap().push(project_id);
        Ok(())
    }
    async fn prepare_worker_directories(&self, _: &Environment, _: Uuid, _: Uuid, _: Uuid) -> Result<WorkerDirectories, AttachmentError> { unreachable!() }
}

struct Chunks(VecDeque<Vec<u8>>);
#[async_trait]
impl ByteSource for Chunks {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, AttachmentError> {
        Ok(self.0.pop_front())
    }
}
fn chunks(bytes: &[u8]) -> Chunks {
    Chunks(bytes.chunks(7).map(<[u8]>::to_vec).collect())
}

#[derive(Default)]
struct TestCaptureHost {
    jobs: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    failures: Arc<Mutex<Vec<CaptureCleanupFailure>>>,
}
impl CaptureHost for TestCaptureHost {
    fn spawn(&self, task: CaptureTask) {
        let failures = self.failures.clone();
        let job = tokio::spawn(async move {
            if let Err(error) = task.await {
                failures.lock().unwrap().push(error);
            }
        });
        self.jobs.lock().unwrap().push(job);
    }
}
impl TestCaptureHost {
    async fn finish(&self) {
        let jobs = std::mem::take(&mut *self.jobs.lock().unwrap());
        for job in jobs {
            tokio::time::timeout(std::time::Duration::from_secs(2), job)
                .await
                .unwrap()
                .unwrap();
        }
    }
}

struct Fixture {
    service: AttachmentService,
    host: Arc<TestCaptureHost>,
    rows: Arc<Rows>,
    files: Arc<Files>,
    conversations: Arc<Conversations>,
    actor: EnvironmentActor,
    env: Environment,
}
impl Fixture {
    fn new() -> Self {
        let actor = EnvironmentActor {
            user_id: Uuid::new_v4(),
            session_hash: CredentialHash([1; 32]),
        };
        let config = EnvironmentStart {
            installation_id: Uuid::new_v4(),
            image_id: "image".into(),
            policy_version: "1".into(),
        };
        let availability = EnvironmentAvailability::default();
        availability.ready();
        let env = Environment {
            id: Uuid::new_v4(),
            user_id: actor.user_id,
            installation_id: config.installation_id,
            desired_state: DesiredState::Running,
            observed_state: ObservedState::Running,
            generation: 1,
            provisioning_state: ProvisioningState::VolumeReady,
            volume_name: "home".into(),
            container_name: "box".into(),
            image_id: "image".into(),
            policy_version: "1".into(),
            observed_at: Some(Utc::now()),
            error_code: None,
            next_reconcile_at: Utc::now(),
        };
        let environments = Arc::new(
            EnvironmentService::new(Arc::new(Env(env.clone())), config, availability).unwrap(),
        );
        let conversations = Arc::new(Conversations {
            conversation: Conversation {
                id: Uuid::new_v4(),
                user_id: actor.user_id,
                project_id: None,
                provider_thread: None,
                created_at: Utc::now(),
            },
            messages: Mutex::default(),
            fail_append: Mutex::default(),
        });
        let rows = Arc::new(Rows {
            conversation: conversations.conversation.id,
            ..Rows::default()
        });
        let host = Arc::new(TestCaptureHost::default());
        let files = Arc::new(Files::default());
        let service = AttachmentService::new(
            Arc::new(ConversationService::new(
                environments,
                conversations.clone(),
            )),
            rows.clone(),
            files.clone(),
            host.clone(),
        );
        Self {
            service,
            host,
            rows,
            files,
            conversations,
            actor,
            env,
        }
    }
    fn conversation(&self) -> Uuid {
        self.conversation.id
    }
    async fn upload(&self, name: &str, bytes: &[u8]) -> Result<Attachment, AttachmentError> {
        self.service
            .upload(self.actor, self.conversation(), name, &mut chunks(bytes))
            .await
    }
}
impl std::ops::Deref for Fixture {
    type Target = Conversations;
    fn deref(&self) -> &Conversations {
        &self.conversations
    }
}

#[tokio::test]
async fn an_upload_is_classified_by_content_staged_and_readable_by_its_owner() {
    let f = Fixture::new();
    let staged = f.upload("photo.txt", PNG).await.unwrap();
    assert_eq!(
        (staged.state, staged.kind, staged.media_type.as_str()),
        (AttachmentState::Staged, MediaKind::Image, "image/png")
    );
    assert_eq!(f.files.names().len(), 1);

    let (_, bytes) = f.service.read(f.actor, staged.id, 2, 3).await.unwrap();
    assert_eq!(bytes, &PNG[2..5]);
    let (_, tail) = f
        .service
        .read(f.actor, staged.id, 4, u64::MAX)
        .await
        .unwrap();
    assert_eq!(tail, &PNG[4..]);
    assert!(
        f.service
            .read(f.actor, staged.id, PNG.len() as u64, 1)
            .await
            .unwrap()
            .1
            .is_empty()
    );
    assert_eq!(
        f.service
            .read(f.actor, staged.id, PNG.len() as u64 + 1, 1)
            .await
            .unwrap_err(),
        AttachmentError::Invalid
    );

    let stranger = EnvironmentActor {
        user_id: Uuid::new_v4(),
        ..f.actor
    };
    assert_eq!(
        f.service.read(stranger, staged.id, 0, 1).await.unwrap_err(),
        AttachmentError::Unauthorized
    );
    assert_eq!(
        f.service
            .upload(stranger, f.conversation(), "a", &mut chunks(b"x"))
            .await
            .unwrap_err(),
        AttachmentError::Unauthorized
    );
}

#[tokio::test]
async fn names_must_be_one_path_component() {
    let f = Fixture::new();
    assert_eq!(
        f.upload("../escape", b"x").await.unwrap_err(),
        AttachmentError::Invalid
    );
    assert!(f.files.names().is_empty() && f.rows.all().is_empty());
}

#[tokio::test]
async fn unsent_batches_do_not_block_later_messages_but_each_message_still_has_a_limit() {
    let f = Fixture::new();
    let mut ids = Vec::new();
    for index in 0..MAX_ATTACHMENTS_PER_MESSAGE * 2 {
        let staged = f.upload(&format!("{index}.txt"), b"text").await.unwrap();
        ids.push(staged.id);
    }
    assert_eq!(
        f.service
            .staged(
                f.actor,
                f.conversation(),
                &ids[..MAX_ATTACHMENTS_PER_MESSAGE + 1]
            )
            .await
            .unwrap_err(),
        AttachmentError::TooMany
    );
    let ready = f
        .service
        .staged(
            f.actor,
            f.conversation(),
            &ids[MAX_ATTACHMENTS_PER_MESSAGE..],
        )
        .await
        .unwrap();
    assert_eq!(ready.len(), MAX_ATTACHMENTS_PER_MESSAGE);
    assert_eq!(f.rows.all().len(), MAX_ATTACHMENTS_PER_MESSAGE * 2);
}

#[tokio::test]
async fn failed_uploads_leave_no_files_and_do_not_block_later_uploads() {
    let f = Fixture::new();
    *f.files.max_put.lock().unwrap() = Some(3);
    for index in 0..MAX_ATTACHMENTS_PER_MESSAGE {
        assert_eq!(
            f.upload(&format!("big-{index}.bin"), b"0123456789")
                .await
                .unwrap_err(),
            AttachmentError::TooLarge
        );
    }
    assert!(f.files.names().is_empty());
    assert!(
        f.rows
            .all()
            .iter()
            .all(|row| row.state == AttachmentState::Uploading)
    );
    *f.files.max_put.lock().unwrap() = None;
    f.upload("small.txt", b"ok").await.unwrap();
    assert_eq!(f.rows.all().len(), MAX_ATTACHMENTS_PER_MESSAGE + 1);
}

#[tokio::test]
async fn staged_uploads_older_than_a_day_are_purged_by_the_next_upload() {
    let f = Fixture::new();
    let old = f.upload("old.txt", b"old").await.unwrap();
    f.rows
        .change(old.id, |a| a.created_at = Utc::now() - Duration::hours(25))
        .unwrap();
    let fresh = f.upload("new.txt", b"new").await.unwrap();
    let ids: Vec<Uuid> = f.rows.all().iter().map(|a| a.id).collect();
    assert_eq!(ids, [fresh.id]);
    assert_eq!(f.files.names().len(), 1);
    assert!(f.files.names()[0].contains(&fresh.id.to_string()));
}

#[tokio::test]
async fn only_staged_files_can_be_removed_and_their_files_go_with_them() {
    let f = Fixture::new();
    let staged = f.upload("a.txt", b"a").await.unwrap();
    let attached = f.upload("b.txt", b"b").await.unwrap();
    f.rows
        .change(attached.id, |a| a.state = AttachmentState::Attached)
        .unwrap();
    assert_eq!(
        f.service
            .remove_staged(f.actor, attached.id)
            .await
            .unwrap_err(),
        AttachmentError::Conflict
    );
    f.service.remove_staged(f.actor, staged.id).await.unwrap();
    assert_eq!(f.files.names().len(), 1);
    assert_eq!(
        f.service
            .remove_staged(f.actor, staged.id)
            .await
            .unwrap_err(),
        AttachmentError::NotFound
    );
}

#[tokio::test]
async fn a_turn_can_only_use_staged_files_of_its_own_conversation() {
    let f = Fixture::new();
    let staged = f.upload("a.txt", b"a").await.unwrap();
    assert_eq!(
        f.service
            .staged(f.actor, f.conversation(), &[staged.id])
            .await
            .unwrap(),
        std::slice::from_ref(&staged)
    );
    assert_eq!(
        f.service
            .staged(f.actor, Uuid::new_v4(), &[staged.id])
            .await
            .unwrap_err(),
        AttachmentError::NotFound
    );
    assert_eq!(
        f.service
            .staged(f.actor, f.conversation(), &[staged.id, staged.id])
            .await
            .unwrap_err(),
        AttachmentError::Invalid
    );
    f.rows
        .change(staged.id, |a| a.state = AttachmentState::Attached)
        .unwrap();
    assert_eq!(
        f.service
            .staged(f.actor, f.conversation(), &[staged.id])
            .await
            .unwrap_err(),
        AttachmentError::Conflict
    );
}

#[tokio::test]
async fn project_purge_is_repeatable() {
    let f = Fixture::new();
    let project = Uuid::new_v4();
    f.service.purge_project(f.actor, project).await.unwrap();
    f.service.purge_project(f.actor, project).await.unwrap();
    assert_eq!(*f.files.purged_projects.lock().unwrap(), [project, project]);
}

#[tokio::test]
async fn concurrency_is_bounded_per_user_and_entries_are_reclaimed() {
    let gates = Gates::new(2);
    let (alice, bob) = (Uuid::new_v4(), Uuid::new_v4());
    let held = [
        gates.try_acquire(alice).unwrap(),
        gates.try_acquire(alice).unwrap(),
    ];
    assert_eq!(
        gates.try_acquire(alice).unwrap_err(),
        AttachmentError::Unavailable
    );
    let _other = gates.try_acquire(bob).unwrap();
    let waiting = gates.acquire(alice);
    drop(held);
    drop(waiting.await.unwrap());
    drop(_other);
    drop(gates.try_acquire(Uuid::new_v4()).unwrap());
    assert_eq!(gates.tracked_users(), 1);
    let busy: Vec<_> = (0..256)
        .map(|_| gates.try_acquire(Uuid::new_v4()).unwrap())
        .collect();
    assert_eq!(
        gates.try_acquire(Uuid::new_v4()).unwrap_err(),
        AttachmentError::Unavailable
    );
    drop(busy);
}

fn finished() -> Deadline {
    Box::pin(async {})
}

#[tokio::test]
async fn an_agent_message_keeps_its_files_without_the_block_and_marks_unreachable_ones() {
    let f = Fixture::new();
    let scope = AgentScope::chat(None, f.conversation());
    f.files
        .sources
        .lock()
        .unwrap()
        .insert("/home/factory/Documents/out/plot.png".into(), PNG.to_vec());
    let run = Uuid::new_v4();
    let text = "Here it is.\n\n```factory-files\nout/plot.png\n/etc/passwd\nmissing.txt\n```";
    let append = |deadline| {
        f.service
            .append_message(f.actor, &f.env, run, &scope, "item-1", text, deadline)
    };
    // The deadline only matters once a capture does not finish.
    append(Box::pin(pending())).await.unwrap();
    assert_eq!(
        f.conversations.messages.lock().unwrap()[0].1,
        "item-1:Here it is."
    );
    let rows = f.rows.all();
    let states: Vec<_> = rows.iter().map(|a| (a.name.as_str(), a.state)).collect();
    assert_eq!(
        states,
        [
            ("plot.png", AttachmentState::Attached),
            ("passwd", AttachmentState::Unavailable),
            ("missing.txt", AttachmentState::Unavailable)
        ]
    );
    assert_eq!(rows[0].kind, MediaKind::Image);
    // Only the file inside the allowed roots was ever copied, from its resolved path.
    let captures = f.files.captures.lock().unwrap().clone();
    assert_eq!(
        captures.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        [
            "/home/factory/Documents/out/plot.png",
            "/home/factory/Documents/missing.txt"
        ]
    );
    assert!(captures[0].1.contains(&format!(
        "/home/factory/.factory/attachments/chats/{}",
        f.conversation()
    )));
    // Replaying the persisted message does not capture or re-hash its bytes.
    append(Box::pin(pending())).await.unwrap();
    assert_eq!(f.rows.all(), rows);
    assert_eq!(f.files.captures.lock().unwrap().len(), captures.len());
    f.host.finish().await;
    assert_ne!(derived_id(run, "item-1", 1), derived_id(run, "item-1", 0));
    assert_ne!(derived_id(run, "item-2", 0), derived_id(run, "item-1", 0));
}

#[tokio::test]
async fn a_message_that_only_lists_files_is_stored_under_their_names() {
    let f = Fixture::new();
    let scope = AgentScope::chat(None, f.conversation());
    f.files
        .sources
        .lock()
        .unwrap()
        .insert("/home/factory/Documents/a.txt".into(), b"a".to_vec());
    let text = "```factory-files\na.txt\n```";
    f.service
        .append_message(
            f.actor,
            &f.env,
            Uuid::new_v4(),
            &scope,
            "item",
            text,
            Box::pin(pending()),
        )
        .await
        .unwrap();
    assert_eq!(f.conversations.messages.lock().unwrap()[0].1, "item:a.txt");
}

#[tokio::test]
async fn capture_backpressure_preserves_the_completed_reply() {
    for full_registry in [false, true] {
        let f = Fixture::new();
        let scope = AgentScope::chat(None, f.conversation());
        let run = Uuid::new_v4();
        let held = if full_registry {
            (0..256)
                .map(|_| f.service.captures.try_acquire(Uuid::new_v4()).unwrap())
                .collect::<Vec<_>>()
        } else {
            vec![
                f.service
                    .captures
                    .try_acquire(derived_id(run, "backpressure", usize::MAX))
                    .unwrap(),
            ]
        };
        let deadline: Deadline = if full_registry {
            Box::pin(pending())
        } else {
            finished()
        };
        f.service
            .append_message(
                f.actor,
                &f.env,
                run,
                &scope,
                "backpressure",
                "Finished reply.\n```factory-files\na.txt\nb.txt\n```",
                deadline,
            )
            .await
            .unwrap();
        assert_eq!(
            f.conversations.messages.lock().unwrap()[0].1,
            "backpressure:Finished reply."
        );
        let rows = f.rows.all();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, "a.txt");
        assert_eq!(rows[1].name, "b.txt");
        assert!(rows.iter().all(|a| a.state == AttachmentState::Unavailable));
        assert!(f.files.captures.lock().unwrap().is_empty());
        f.host.finish().await;
        assert!(f.host.failures.lock().unwrap().is_empty());
        drop(held);
    }
}

#[tokio::test]
async fn a_late_capture_is_cleaned_after_it_really_finishes() {
    let f = Fixture::new();
    *f.files.stall.lock().unwrap() = true;
    f.files.sources.lock().unwrap().insert(
        "/home/factory/Documents/a.txt".into(),
        b"late bytes".to_vec(),
    );
    let scope = AgentScope::chat(None, f.conversation());
    let (expire, expired) = tokio::sync::oneshot::channel::<()>();
    let (result, ()) = tokio::join!(
        f.service.append_message(
            f.actor,
            &f.env,
            Uuid::new_v4(),
            &scope,
            "late",
            "```factory-files\na.txt\nb.txt\n```",
            Box::pin(async {
                let _ = expired.await;
            }),
        ),
        async {
            f.files.copy_started.notified().await;
            expire.send(()).unwrap();
        }
    );
    result.unwrap();
    assert!(
        f.rows
            .all()
            .iter()
            .all(|file| file.state == AttachmentState::Unavailable)
    );
    assert_eq!(f.files.captures.lock().unwrap().len(), 1);
    f.files.copy_release.notify_one();
    f.host.finish().await;
    assert!(f.files.names().is_empty());
    assert!(f.host.failures.lock().unwrap().is_empty());
}

#[tokio::test]
async fn dropping_the_caller_does_not_abandon_its_started_copy() {
    let f = Fixture::new();
    let scope = AgentScope::chat(None, f.conversation());
    *f.files.stall.lock().unwrap() = true;
    f.files.sources.lock().unwrap().insert(
        "/home/factory/Documents/a.txt".into(),
        b"late bytes".to_vec(),
    );
    let mut request = Box::pin(f.service.append_message(
        f.actor,
        &f.env,
        Uuid::new_v4(),
        &scope,
        "cancel",
        "```factory-files\na.txt\n```",
        Box::pin(pending()),
    ));
    tokio::select! {
        _ = f.files.copy_started.notified() => {},
        result = &mut request => panic!("copy should still be active: {result:?}"),
    }
    drop(request);
    f.files.copy_release.notify_one();
    f.host.finish().await;
    assert!(f.files.names().is_empty());
    assert!(f.rows.all().is_empty());
    assert!(f.conversations.messages.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cleanup_failures_are_reported_with_identifiers_and_no_content() {
    let f = Fixture::new();
    let scope = AgentScope::chat(None, f.conversation());
    f.files.sources.lock().unwrap().insert(
        "/home/factory/Documents/a.txt".into(),
        b"uncommitted".to_vec(),
    );
    *f.rows.fail_record.lock().unwrap() = true;
    *f.files.delete_error.lock().unwrap() = Some(AttachmentError::Unavailable);
    let run = Uuid::new_v4();
    assert_eq!(
        f.service
            .append_message(
                f.actor,
                &f.env,
                run,
                &scope,
                "item",
                "```factory-files\na.txt\n```",
                Box::pin(pending())
            )
            .await,
        Err(ConversationError::Unavailable)
    );
    f.host.finish().await;
    let failures = f.host.failures.lock().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].environment_id, f.env.id);
    assert_eq!(failures[0].attachment_id, derived_id(run, "item", 0));
    assert_eq!(failures[0].error, AttachmentError::Unavailable);
}

#[tokio::test]
async fn uncertain_process_completion_is_reported_without_racing_deletion() {
    let f = Fixture::new();
    let scope = AgentScope::chat(None, f.conversation());
    *f.files.capture_error.lock().unwrap() = Some(AttachmentError::Unavailable);
    *f.files.delete_error.lock().unwrap() = Some(AttachmentError::Conflict);
    f.service
        .append_message(
            f.actor,
            &f.env,
            Uuid::new_v4(),
            &scope,
            "item",
            "```factory-files\na.txt\n```",
            Box::pin(pending()),
        )
        .await
        .unwrap();
    f.host.finish().await;
    assert_eq!(f.rows.all()[0].state, AttachmentState::Unavailable);
    let failures = f.host.failures.lock().unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].error, AttachmentError::Unavailable);
}

#[tokio::test]
async fn a_replay_preserves_stored_state_without_recapturing() {
    let f = Fixture::new();
    let scope = AgentScope::chat(None, f.conversation());
    f.files.sources.lock().unwrap().insert(
        "/home/factory/Documents/a.txt".into(),
        b"first capture".to_vec(),
    );
    let run = Uuid::new_v4();
    let text = "```factory-files\na.txt\nmissing.txt\n```";
    f.service
        .append_message(
            f.actor,
            &f.env,
            run,
            &scope,
            "item",
            text,
            Box::pin(pending()),
        )
        .await
        .unwrap();
    f.host.finish().await;
    let recorded = f.rows.all();
    assert_eq!(
        recorded.iter().map(|a| a.state).collect::<Vec<_>>(),
        [AttachmentState::Attached, AttachmentState::Unavailable]
    );
    let count = f.files.captures.lock().unwrap().len();
    f.files.sources.lock().unwrap().clear();
    *f.files.stall.lock().unwrap() = true;
    f.service
        .append_message(f.actor, &f.env, run, &scope, "item", text, finished())
        .await
        .unwrap();
    assert_eq!(f.rows.all(), recorded);
    assert_eq!(f.files.captures.lock().unwrap().len(), count);
    assert_eq!(f.files.names().len(), 1);
    assert_eq!(f.conversations.messages.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_metadata_persistence_cleans_the_uncommitted_capture() {
    let f = Fixture::new();
    let scope = AgentScope::chat(None, f.conversation());
    f.files.sources.lock().unwrap().insert(
        "/home/factory/Documents/a.txt".into(),
        b"uncommitted".to_vec(),
    );
    *f.rows.fail_record.lock().unwrap() = true;
    assert_eq!(
        f.service
            .append_message(
                f.actor,
                &f.env,
                Uuid::new_v4(),
                &scope,
                "item",
                "```factory-files\na.txt\n```",
                Box::pin(pending())
            )
            .await,
        Err(ConversationError::Unavailable)
    );
    f.host.finish().await;
    assert!(f.files.names().is_empty());
    assert!(f.rows.all().is_empty());
}

#[tokio::test]
async fn a_lost_commit_reply_never_deletes_the_attached_file() {
    let f = Fixture::new();
    let scope = AgentScope::chat(None, f.conversation());
    f.files.sources.lock().unwrap().insert(
        "/home/factory/Documents/a.txt".into(),
        b"committed".to_vec(),
    );
    *f.rows.fail_after_commit.lock().unwrap() = true;
    assert_eq!(
        f.service
            .append_message(
                f.actor,
                &f.env,
                Uuid::new_v4(),
                &scope,
                "item",
                "```factory-files\na.txt\n```",
                Box::pin(pending())
            )
            .await,
        Err(ConversationError::Unavailable)
    );
    f.host.finish().await;
    assert_eq!(f.rows.all()[0].state, AttachmentState::Attached);
    assert_eq!(f.files.names().len(), 1);
    assert!(f.host.failures.lock().unwrap().is_empty());
}

#[tokio::test]
async fn concurrent_replays_share_the_canonical_capture() {
    let f = Fixture::new();
    let scope = AgentScope::chat(None, f.conversation());
    f.files
        .sources
        .lock()
        .unwrap()
        .insert("/home/factory/Documents/a.txt".into(), b"original".to_vec());
    *f.files.stall.lock().unwrap() = true;
    let run = Uuid::new_v4();
    let text = "```factory-files\na.txt\n```";
    let (first, replay) = tokio::join!(
        f.service.append_message(
            f.actor,
            &f.env,
            run,
            &scope,
            "item",
            text,
            Box::pin(pending())
        ),
        async {
            f.files.copy_started.notified().await;
            f.files.copy_release.notify_one();
            f.service
                .append_message(
                    f.actor,
                    &f.env,
                    run,
                    &scope,
                    "item",
                    text,
                    Box::pin(pending()),
                )
                .await
        }
    );
    first.unwrap();
    replay.unwrap();
    f.host.finish().await;
    assert_eq!(f.files.captures.lock().unwrap().len(), 1);
    assert_eq!(f.rows.all().len(), 1);
    assert_eq!(f.conversations.messages.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn failed_message_persistence_cleans_the_uncommitted_capture() {
    let f = Fixture::new();
    let scope = AgentScope::chat(None, f.conversation());
    f.files.sources.lock().unwrap().insert(
        "/home/factory/Documents/a.txt".into(),
        b"uncommitted".to_vec(),
    );
    *f.conversations.fail_append.lock().unwrap() = true;
    assert_eq!(
        f.service
            .append_message(
                f.actor,
                &f.env,
                Uuid::new_v4(),
                &scope,
                "item",
                "```factory-files\na.txt\n```",
                Box::pin(pending())
            )
            .await,
        Err(ConversationError::Unavailable)
    );
    f.host.finish().await;
    assert!(f.files.names().is_empty());
}

#[tokio::test]
async fn a_worker_reaches_only_its_mounted_directories() {
    let f = Fixture::new();
    let directories = WorkerDirectories {
        attachments: ".factory/attachments/p/c".into(),
        outputs: ".factory/attachments/p/c/.outputs/a".into(),
    };
    let scope = AgentScope::worker(Uuid::new_v4(), f.conversation(), &directories);
    let output = "/home/factory/.factory/attachments/p/c/.outputs/a/report.html";
    f.files
        .sources
        .lock()
        .unwrap()
        .insert(output.into(), b"<!doctype html><p>x".to_vec());
    let references = [
        "report.html".to_owned(),
        "/workspace/checkout/src/lib.rs".to_owned(),
        "/workspace/outputs/../attachments/x/y".to_owned(),
    ];
    let text = format!("```factory-files\n{}\n```", references.join("\n"));
    f.service
        .append_message(
            f.actor,
            &f.env,
            Uuid::new_v4(),
            &scope,
            "worker",
            &text,
            Box::pin(pending()),
        )
        .await
        .unwrap();
    let kinds: Vec<_> = f
        .rows
        .all()
        .iter()
        .map(|file| file.state == AttachmentState::Attached)
        .collect();
    assert_eq!(kinds, [true, false, false]);
    let captures = f.files.captures.lock().unwrap().clone();
    assert_eq!(captures[0].0, output);
    assert_eq!(
        captures[0].1,
        [
            "/home/factory/.factory/attachments/p/c/.outputs/a",
            "/home/factory/.factory/attachments/p/c"
        ]
    );
    // `..` out of one mount into the other is still inside a root and is mapped.
    assert_eq!(captures[1].0, "/home/factory/.factory/attachments/p/c/x/y");
    f.host.finish().await;
}

#[test]
fn prompts_name_each_file_by_the_path_its_agent_can_open() {
    let id = Uuid::new_v4();
    let conversation = Uuid::new_v4();
    let project = Uuid::new_v4();
    let file = Attachment {
        id,
        conversation_id: conversation,
        run_id: None,
        message_id: None,
        source: AttachmentSource::User,
        state: AttachmentState::Staged,
        name: "a b.png".into(),
        media_type: "image/png".into(),
        kind: MediaKind::Image,
        size: 12,
        created_at: Utc::now(),
    };
    let container = Vantage::Container {
        project_id: Some(project),
    }
    .prompt_suffix(std::slice::from_ref(&file));
    assert!(container.contains(&format!("- /home/factory/.factory/attachments/{project}/{conversation}/{id}/a b.png (image, 12 bytes)")));
    assert!(container.contains("```") || container.contains("factory-files"));
    let worker = Vantage::Worker.prompt_suffix(std::slice::from_ref(&file));
    assert!(worker.contains(&format!(
        "- /workspace/attachments/{id}/a b.png (image, 12 bytes)"
    )));
    assert!(
        worker.contains("/workspace/outputs")
            && worker.contains("before any factory-proposal block")
    );
    assert!(
        !Vantage::Worker
            .prompt_suffix(&[])
            .contains("Files attached")
    );

    let message = Uuid::new_v4();
    let mut sent = file;
    sent.message_id = Some(message);
    sent.state = AttachmentState::Attached;
    let note = Vantage::Worker.history_note(&[sent.clone()], message);
    assert_eq!(
        note,
        format!("Attached files:\n- /workspace/attachments/{id}/a b.png (image, 12 bytes)\n")
    );
    assert_eq!(Vantage::Worker.history_note(&[sent], Uuid::new_v4()), "");
}
