use super::{agent::derived_id, *};
use crate::environments::{EnvironmentAvailability, EnvironmentService};
use async_trait::async_trait;
use chrono::Utc;
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
    async fn append_assistant(&self, _: EnvironmentActor, _: Uuid, _: i64, _: Uuid, item: &str, text: &str) -> Result<(Message, bool), ConversationError> {
        if text.is_empty() {
            return Err(ConversationError::Invalid);
        }
        let message = Message { id: Uuid::new_v4(), run_id: Uuid::nil(), role: MessageRole::Assistant, content: text.to_owned(), created_at: Utc::now() };
        self.messages.lock().unwrap().push((message.id, format!("{item}:{text}")));
        Ok((message, true))
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
struct Rows(Mutex<Vec<Attachment>>);
impl Rows {
    fn all(&self) -> Vec<Attachment> {
        self.0.lock().unwrap().clone()
    }
    fn change(
        &self,
        id: Uuid,
        change: impl FnOnce(&mut Attachment),
    ) -> Result<Attachment, AttachmentError> {
        let mut rows = self.0.lock().unwrap();
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
        self.0.lock().unwrap().push(row.clone());
        Ok(row)
    }
    async fn mark_staged(&self, _: EnvironmentActor, _: Uuid, _: i64, id: Uuid, file: StoredFile) -> Result<Attachment, AttachmentError> {
        self.change(id, |a| (a.state, a.media_type, a.kind, a.size) = (AttachmentState::Staged, file.media_type, file.kind, file.size))
    }
    async fn remove_staged(&self, _: EnvironmentActor, _: Uuid, _: i64, id: Uuid) -> Result<Attachment, AttachmentError> {
        let mut rows = self.0.lock().unwrap();
        let index = rows.iter().position(|a| a.id == id).ok_or(AttachmentError::NotFound)?;
        if rows[index].state != AttachmentState::Staged {
            return Err(AttachmentError::Conflict);
        }
        Ok(rows.remove(index))
    }
    async fn expire_staged(&self, _: EnvironmentActor, before: chrono::DateTime<Utc>) -> Result<Vec<Attachment>, AttachmentError> {
        let mut rows = self.0.lock().unwrap();
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
        let mut rows = self.0.lock().unwrap();
        let mut recorded = Vec::new();
        for file in files {
            let (state, stored) = match file.capture {
                AgentCapture::Captured(stored) => (AttachmentState::Attached, Some(stored)),
                AgentCapture::Unavailable => (AttachmentState::Unavailable, None),
            };
            let row = Attachment {
                id: file.id, conversation_id: Uuid::nil(), run_id: Some(run_id), message_id: Some(message_id), source: AttachmentSource::Agent, state, name: file.name,
                media_type: stored.as_ref().map_or(OCTET_STREAM.into(), |s| s.media_type.clone()), kind: stored.as_ref().map_or(MediaKind::File, |s| s.kind), size: stored.map_or(0, |s| s.size), created_at: Utc::now(),
            };
            rows.push(row.clone());
            recorded.push(row);
        }
        Ok(recorded)
    }
}

#[derive(Default)]
struct Files {
    stored: Mutex<HashMap<String, Vec<u8>>>,
    /// Container files the agent can reference, by absolute path.
    sources: Mutex<HashMap<String, Vec<u8>>>,
    captures: Mutex<Vec<(String, Vec<String>)>>,
    stall: Mutex<bool>,
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
        if *self.stall.lock().unwrap() {
            pending::<()>().await;
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

struct Fixture {
    service: AttachmentService,
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
        });
        let rows = Arc::new(Rows::default());
        let files = Arc::new(Files::default());
        let service = AttachmentService::new(
            Arc::new(ConversationService::new(
                environments,
                conversations.clone(),
            )),
            rows.clone(),
            files.clone(),
        );
        Self {
            service,
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
async fn at_most_ten_files_are_staged_per_conversation() {
    let f = Fixture::new();
    let mut first = None;
    for index in 0..MAX_ATTACHMENTS_PER_MESSAGE {
        let staged = f.upload(&format!("{index}.txt"), b"text").await.unwrap();
        first.get_or_insert(staged.id);
    }
    assert_eq!(
        f.upload("extra.txt", b"text").await.unwrap_err(),
        AttachmentError::TooMany
    );
    f.service
        .remove_staged(f.actor, first.unwrap())
        .await
        .unwrap();
    f.upload("extra.txt", b"text").await.unwrap();
}

#[tokio::test]
async fn a_failed_upload_leaves_no_file_and_does_not_count_once_abandoned() {
    let f = Fixture::new();
    *f.files.max_put.lock().unwrap() = Some(3);
    assert_eq!(
        f.upload("big.bin", b"0123456789").await.unwrap_err(),
        AttachmentError::TooLarge
    );
    assert!(f.files.names().is_empty());
    assert_eq!(f.rows.all()[0].state, AttachmentState::Uploading);
    f.rows
        .change(f.rows.all()[0].id, |a| {
            a.created_at = Utc::now() - Duration::hours(1)
        })
        .unwrap();
    *f.files.max_put.lock().unwrap() = None;
    f.upload("small.txt", b"ok").await.unwrap();
    assert_eq!(f.rows.all().len(), 2);
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
    // Replaying the message reuses the same attachment IDs.
    let again = f
        .service
        .capture_agent_files(
            &f.env,
            &scope,
            run,
            "item-1",
            &["out/plot.png".into()],
            finished(),
        )
        .await;
    assert_eq!(again[0].id, rows[0].id);
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
            finished(),
        )
        .await
        .unwrap();
    assert_eq!(f.conversations.messages.lock().unwrap()[0].1, "item:a.txt");
}

#[tokio::test]
async fn a_capture_past_its_deadline_is_unavailable_and_leaves_nothing_behind() {
    let f = Fixture::new();
    *f.files.stall.lock().unwrap() = true;
    let scope = AgentScope::chat(None, f.conversation());
    let references = ["a.txt".to_owned(), "b.txt".to_owned()];
    let files = f
        .service
        .capture_agent_files(
            &f.env,
            &scope,
            Uuid::new_v4(),
            "item",
            &references,
            finished(),
        )
        .await;
    assert!(
        files
            .iter()
            .all(|file| file.capture == AgentCapture::Unavailable)
    );
    // After the deadline the remaining references are not even attempted.
    assert_eq!(f.files.captures.lock().unwrap().len(), 1);
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
    let files = f
        .service
        .capture_agent_files(
            &f.env,
            &scope,
            Uuid::new_v4(),
            "item",
            &references,
            finished(),
        )
        .await;
    let kinds: Vec<_> = files
        .iter()
        .map(|file| matches!(file.capture, AgentCapture::Captured(_)))
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
