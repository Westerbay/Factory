use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use factory_api::{auth::AuthState, config::RuntimeConfig, conversations};
use factory_application::{
    agent_connections::AgentConnectionService,
    conversations::ConversationService,
    conversations::coordinator::ExecutionProvider,
    environments::{EnvironmentAvailability, EnvironmentReconciler, EnvironmentService},
    identity::IdentityService,
    projects::ProjectService,
    terminals::TerminalService,
};
use factory_core::{
    agent_connections::*,
    conversations::{execution::*, *},
    environments::*,
    identity::*,
    projects::*,
    terminals::*,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, Notify, mpsc};
use tower::ServiceExt;
use uuid::Uuid;

#[path = "conversations/features.rs"]
mod features;

const ORIGIN: &str = "http://localhost:3000";
const CSRF: &str = "fixture-account-csrf";
struct Store {
    sessions: Mutex<HashMap<CredentialHash, ActiveSession>>,
    environment: Mutex<Environment>,
    environment_reads: AtomicUsize,
}

#[async_trait]
impl IdentityStore for Store {
    async fn create_login_attempt(&self, _: NewLoginAttempt) -> Result<(), IdentityError> {
        unreachable!()
    }
    async fn take_login_attempt(
        &self,
        _: CredentialHash,
        _: CredentialHash,
    ) -> Result<LoginAttempt, IdentityError> {
        unreachable!()
    }
    async fn admit(&self, _: LoginAdmission) -> Result<AdmissionOutcome, IdentityError> {
        unreachable!()
    }
    async fn active_session(
        &self,
        hash: CredentialHash,
    ) -> Result<Option<ActiveSession>, IdentityError> {
        Ok(self
            .sessions
            .lock()
            .unwrap()
            .get(&hash)
            .filter(|session| session.expires_at > Utc::now())
            .cloned())
    }
    async fn logout(&self, hash: CredentialHash) -> Result<(), IdentityError> {
        self.sessions.lock().unwrap().remove(&hash);
        Ok(())
    }
    async fn list_users(
        &self,
        _: CredentialHash,
        _: UserListQuery,
    ) -> Result<UserPage, IdentityError> {
        unreachable!()
    }
    async fn change_access(
        &self,
        _: CredentialHash,
        _: AccessRequest,
    ) -> Result<User, IdentityError> {
        unreachable!()
    }
}

struct NoProvider;
#[async_trait]
impl IdentityProvider for NoProvider {
    fn provider(&self) -> &str {
        "github"
    }
    fn issuer(&self) -> &str {
        "https://github.com"
    }
    fn authorization_url(&self, _: &str, _: &str) -> Result<String, IdentityError> {
        unreachable!()
    }
    async fn exchange(&self, _: &str, _: &str) -> Result<VerifiedIdentity, IdentityError> {
        unreachable!()
    }
}

#[async_trait]
impl EnvironmentStore for Store {
    async fn get(&self, actor: EnvironmentActor) -> Result<Option<Environment>, EnvironmentError> {
        self.environment_reads.fetch_add(1, Ordering::SeqCst);
        let sessions = self.sessions.lock().unwrap();
        if !sessions.get(&actor.session_hash).is_some_and(|session| {
            session.user.id == actor.user_id
                && session.user.status == AccessStatus::Active
                && session.expires_at > Utc::now()
        }) {
            return Err(EnvironmentError::Unauthorized);
        }
        let environment = self.environment.lock().unwrap();
        let mut environment = environment.clone();
        environment.user_id = actor.user_id;
        Ok(Some(environment))
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
        Ok(vec![])
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

struct Supervisor;
#[async_trait]
impl EnvironmentSupervisor for Supervisor {
    async fn check(&mut self) -> Result<(), EnvironmentError> {
        Ok(())
    }
}
struct NoEnvironmentRuntime;
#[async_trait]
impl EnvironmentRuntime for NoEnvironmentRuntime {
    async fn inspect(&self, _: &Environment) -> Result<RuntimeInspection, EnvironmentError> {
        unreachable!()
    }
    async fn create_volume(&self, _: &Environment) -> Result<(), EnvironmentError> {
        unreachable!()
    }
    async fn start(&self, _: &Environment) -> Result<(), EnvironmentError> {
        unreachable!()
    }
    async fn stop(&self, _: &Environment) -> Result<(), EnvironmentError> {
        unreachable!()
    }
}

struct Connected(bool);
#[async_trait]
impl AgentConnectionPort for Connected {
    async fn read(&self, _: &Environment) -> Result<ConnectionSnapshot, AgentConnectionError> {
        Ok(ConnectionSnapshot::account(
            self.0.then(|| AccountSummary {
                email: None,
                plan: "fixture".into(),
            }),
            Utc::now(),
        ))
    }
    async fn begin_login(
        &self,
        _: &Environment,
        _: Uuid,
        _: DateTime<Utc>,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        unreachable!()
    }
    async fn cancel_login(
        &self,
        _: &Environment,
        _: Uuid,
    ) -> Result<ConnectionSnapshot, AgentConnectionError> {
        unreachable!()
    }
    async fn logout(&self, _: &Environment) -> Result<ConnectionSnapshot, AgentConnectionError> {
        unreachable!()
    }
}
#[derive(Default)]
struct ConversationData {
    conversation: Option<Conversation>,
    run: Option<Run>,
    run_environment: Option<Uuid>,
    messages: Vec<Message>,
    probes: std::collections::HashMap<Uuid, (Uuid, Uuid, i64, bool)>,
    run_settled: bool,
}
#[derive(Default)]
struct Conversations(Mutex<ConversationData>, AtomicUsize, Mutex<Option<Project>>);
impl Conversations {
    fn owned(&self, actor: EnvironmentActor, id: Uuid) -> Result<Conversation, ConversationError> {
        self.0
            .lock()
            .unwrap()
            .conversation
            .clone()
            .filter(|c| c.id == id && c.user_id == actor.user_id)
            .ok_or(ConversationError::NotFound)
    }
}
#[async_trait]
impl ConversationStore for Conversations {
    async fn reserve_model_probe(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        lease: Uuid,
    ) -> Result<(), ConversationError> {
        let mut d = self.0.lock().unwrap();
        if d.probes
            .values()
            .any(|(user, _, _, _)| *user == actor.user_id)
        {
            return Err(ConversationError::Conflict);
        }
        d.probes
            .insert(lease, (actor.user_id, env, generation, false));
        Ok(())
    }
    async fn settle_model_probe_launch(
        &self,
        lease: Uuid,
        env: Uuid,
        generation: i64,
    ) -> Result<(), ConversationError> {
        let mut d = self.0.lock().unwrap();
        let value = d
            .probes
            .get_mut(&lease)
            .ok_or(ConversationError::Conflict)?;
        if value.1 != env || value.2 != generation {
            return Err(ConversationError::Conflict);
        }
        value.3 = true;
        Ok(())
    }
    async fn release_model_probe(
        &self,
        actor: EnvironmentActor,
        lease: Uuid,
    ) -> Result<(), ConversationError> {
        let mut d = self.0.lock().unwrap();
        if d.probes
            .get(&lease)
            .is_none_or(|(user, _, _, settled)| *user != actor.user_id || !settled)
        {
            return Err(ConversationError::Conflict);
        }
        d.probes.remove(&lease);
        Ok(())
    }
    async fn recover_model_probe_stopped(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        _: chrono::DateTime<Utc>,
    ) -> Result<bool, ConversationError> {
        let mut d = self.0.lock().unwrap();
        let before = d.probes.len();
        d.probes.retain(|_, (user, old_env, old_gen, settled)| {
            *user != actor.user_id || *old_env != env || *old_gen >= generation || !*settled
        });
        if d.probes
            .values()
            .any(|(user, old_env, _, _)| *user == actor.user_id && *old_env == env)
        {
            return Err(ConversationError::Conflict);
        }
        Ok(before != d.probes.len())
    }
    async fn has_open_runs(&self, actor: EnvironmentActor) -> Result<bool, ConversationError> {
        let data = self.0.lock().unwrap();
        Ok(data
            .probes
            .values()
            .any(|(user, _, _, _)| *user == actor.user_id)
            || data
                .conversation
                .as_ref()
                .is_some_and(|c| c.user_id == actor.user_id)
                && data
                    .run
                    .as_ref()
                    .is_some_and(|r| matches!(r.status, RunStatus::Active | RunStatus::Uncertain)))
    }
    async fn create(
        &self,
        actor: EnvironmentActor,
        _: Uuid,
        _: i64,
    ) -> Result<Conversation, ConversationError> {
        let c = Conversation {
            id: Uuid::new_v4(),
            user_id: actor.user_id,
            project_id: None,
            provider_thread: None,
            created_at: Utc::now(),
        };
        self.0.lock().unwrap().conversation = Some(c.clone());
        Ok(c)
    }
    async fn create_in_project(
        &self,
        actor: EnvironmentActor,
        environment_id: Uuid,
        generation: i64,
        project_id: Option<Uuid>,
    ) -> Result<Conversation, ConversationError> {
        if let Some(id) = project_id {
            let guard = self.2.lock().unwrap();
            let valid = guard.as_ref().is_some_and(|project| {
                project.id == id
                    && project.user_id == actor.user_id
                    && project.environment_id == environment_id
                    && project.state == ProjectState::Ready
                    && generation == 1
            });
            if !valid {
                return Err(ConversationError::NotFound);
            }
        }
        let conversation = Conversation {
            id: Uuid::new_v4(),
            user_id: actor.user_id,
            project_id,
            provider_thread: None,
            created_at: Utc::now(),
        };
        self.0.lock().unwrap().conversation = Some(conversation.clone());
        Ok(conversation)
    }
    async fn list(
        &self,
        actor: EnvironmentActor,
        _: Uuid,
        _: i64,
        before: Option<Uuid>,
    ) -> Result<ConversationPage, ConversationError> {
        let items = self
            .0
            .lock()
            .unwrap()
            .conversation
            .clone()
            .into_iter()
            .filter(|c| c.user_id == actor.user_id && Some(c.id) != before)
            .collect();
        Ok(ConversationPage {
            items,
            next_cursor: None,
        })
    }
    async fn history(
        &self,
        actor: EnvironmentActor,
        _: Uuid,
        _: i64,
        id: Uuid,
    ) -> Result<ConversationHistory, ConversationError> {
        self.1.fetch_add(1, Ordering::SeqCst);
        self.owned(actor, id)?;
        let d = self.0.lock().unwrap();
        Ok(ConversationHistory {
            conversation: d.conversation.clone().unwrap(),
            runs: d.run.clone().into_iter().collect(),
            messages: d.messages.clone(),
            requests: vec![],
            runs_truncated: false,
            messages_truncated: false,
            requests_truncated: false,
        })
    }
    async fn find_turn(
        &self,
        actor: EnvironmentActor,
        turn: StartTurn,
    ) -> Result<Option<Run>, ConversationError> {
        self.owned(actor, turn.conversation_id)?;
        let d = self.0.lock().unwrap();
        let Some(run) = d
            .run
            .as_ref()
            .filter(|run| run.operation_id == turn.operation_id)
        else {
            return Ok(None);
        };
        if run.model != turn.model
            || run.environment_generation != turn.generation
            || d.run_environment != Some(turn.environment_id)
            || !d.messages.iter().any(|message| {
                message.run_id == run.id
                    && message.role == MessageRole::User
                    && message.content == turn.input
            })
        {
            return Err(ConversationError::Conflict);
        }
        Ok(Some(run.clone()))
    }
    async fn begin_turn(
        &self,
        actor: EnvironmentActor,
        turn: StartTurn,
    ) -> Result<(Run, bool), ConversationError> {
        self.owned(actor, turn.conversation_id)?;
        let mut d = self.0.lock().unwrap();
        if let Some(r) = &d.run {
            return if r.operation_id == turn.operation_id && r.model == turn.model {
                Ok((r.clone(), false))
            } else {
                Err(ConversationError::Conflict)
            };
        }
        let r = Run {
            id: Uuid::new_v4(),
            conversation_id: turn.conversation_id,
            operation_id: turn.operation_id,
            environment_generation: turn.generation,
            model: turn.model,
            status: RunStatus::Active,
        };
        d.messages.push(Message {
            id: Uuid::new_v4(),
            run_id: r.id,
            role: MessageRole::User,
            content: turn.input,
            created_at: Utc::now(),
        });
        d.run_environment = Some(turn.environment_id);
        d.run = Some(r.clone());
        Ok((r, true))
    }
    async fn settle_run_launch(&self, run: Uuid, _: Uuid, _: i64) -> Result<(), ConversationError> {
        let mut data = self.0.lock().unwrap();
        if data.run.as_ref().is_none_or(|r| r.id != run) {
            return Err(ConversationError::Conflict);
        }
        data.run_settled = true;
        Ok(())
    }
    async fn set_thread(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Uuid,
        thread: &str,
        _: Option<&str>,
    ) -> Result<(), ConversationError> {
        self.0
            .lock()
            .unwrap()
            .conversation
            .as_mut()
            .unwrap()
            .provider_thread = Some(thread.into());
        Ok(())
    }
    async fn append_assistant(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        run: Uuid,
        _: &str,
        text: &str,
    ) -> Result<(Message, bool), ConversationError> {
        let m = Message {
            id: Uuid::new_v4(),
            run_id: run,
            role: MessageRole::Assistant,
            content: text.into(),
            created_at: Utc::now(),
        };
        self.0.lock().unwrap().messages.push(m.clone());
        Ok((m, true))
    }
    async fn add_request(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: PendingRequest,
        _: &str,
    ) -> Result<(), ConversationError> {
        unreachable!()
    }
    async fn claim_response(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Uuid,
        _: Uuid,
        _: &str,
    ) -> Result<ResponseClaim, ConversationError> {
        unreachable!()
    }
    async fn record_response(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Uuid,
        _: Uuid,
        _: ResponseState,
    ) -> Result<(), ConversationError> {
        unreachable!()
    }
    async fn recoverable_stopped(
        &self,
        actor: EnvironmentActor,
        _: Uuid,
        generation: i64,
        _: DateTime<Utc>,
    ) -> Result<Vec<RecoverableRun>, ConversationError> {
        let data = self.0.lock().unwrap();
        let Some(conversation) = data
            .conversation
            .as_ref()
            .filter(|c| c.user_id == actor.user_id)
        else {
            return Ok(vec![]);
        };
        Ok(data
            .run
            .as_ref()
            .filter(|run| {
                run.environment_generation < generation
                    && matches!(run.status, RunStatus::Active | RunStatus::Uncertain)
            })
            .map(|run| RecoverableRun {
                conversation_id: conversation.id,
                run_id: run.id,
                launch_settled: data.run_settled,
            })
            .into_iter()
            .collect())
    }
    async fn recover_stopped(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: DateTime<Utc>,
        _: Uuid,
    ) -> Result<(), ConversationError> {
        unreachable!()
    }
    async fn finish_turn(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Uuid,
        status: RunStatus,
    ) -> Result<(), ConversationError> {
        self.0.lock().unwrap().run.as_mut().unwrap().status = status;
        Ok(())
    }
}
#[async_trait]
impl ProjectStore for Conversations {
    async fn reserve(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: ImportProject,
    ) -> Result<(Project, bool), ProjectError> {
        unreachable!()
    }
    async fn get(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        id: Uuid,
    ) -> Result<Project, ProjectError> {
        self.2
            .lock()
            .unwrap()
            .clone()
            .filter(|project| {
                project.id == id
                    && project.user_id == actor.user_id
                    && project.environment_id == env
                    && generation == 1
            })
            .ok_or(ProjectError::NotFound)
    }
    async fn list(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Option<Uuid>,
    ) -> Result<ProjectPage, ProjectError> {
        unreachable!()
    }
    async fn sync_repository_metadata(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: &[RepositoryMetadata],
    ) -> Result<Vec<i64>, ProjectError> {
        unreachable!()
    }
    async fn settle(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Uuid,
        _: Uuid,
        _: bool,
    ) -> Result<Project, ProjectError> {
        unreachable!()
    }
    async fn recover_stopped(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: DateTime<Utc>,
        _: Uuid,
    ) -> Result<Project, ProjectError> {
        unreachable!()
    }
    async fn recoverable(
        &self,
        _: EnvironmentActor,
    ) -> Result<Vec<RecoverableImport>, ProjectError> {
        unreachable!()
    }
    async fn confirm_recovered(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Uuid,
        _: Uuid,
    ) -> Result<Project, ProjectError> {
        unreachable!()
    }
    async fn retry(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Uuid,
        _: Uuid,
        _: Uuid,
    ) -> Result<Project, ProjectError> {
        unreachable!()
    }
}

struct Control {
    started: Arc<AtomicUsize>,
    stopped: Arc<AtomicUsize>,
}
#[async_trait]
impl ExecutionControl for Control {
    async fn begin_turn(&mut self) -> Result<(), ExecutionError> {
        self.started.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    async fn answer(&mut self, _: Uuid, _: Vec<QuestionAnswer>) -> Result<(), ExecutionError> {
        unreachable!()
    }
    async fn interrupt(&mut self) -> Result<(), ExecutionError> {
        self.stopped.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct Output(mpsc::Receiver<ExecutionEvent>);
#[async_trait]
impl ExecutionOutput for Output {
    async fn next(&mut self) -> Result<Option<ExecutionEvent>, ExecutionError> {
        Ok(self.0.recv().await)
    }
}
struct Provider {
    tx: mpsc::Sender<ExecutionEvent>,
    rx: AsyncMutex<Option<mpsc::Receiver<ExecutionEvent>>>,
    opens: AtomicUsize,
    effort: Mutex<Option<String>>,
    fail_open: Mutex<Option<ExecutionError>>,
    model_started: AtomicUsize,
    fail_models: Mutex<Option<ExecutionError>>,
    hold_models: AtomicBool,
    release_models: Notify,
    started: Arc<AtomicUsize>,
    stopped: Arc<AtomicUsize>,
    project_opens: Mutex<Vec<(Uuid, String, Option<String>)>>,
    project_missing: AtomicBool,
}
#[async_trait]
impl ExecutionRuntime for Provider {
    async fn models(&self, _: &Environment) -> Result<Vec<ExecutionModel>, ExecutionError> {
        self.model_started.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = *self.fail_models.lock().unwrap() {
            return Err(error);
        }
        if self.hold_models.load(Ordering::SeqCst) {
            self.release_models.notified().await;
        }
        Ok(vec![ExecutionModel {
            id: "fixture-model".into(),
            name: "Fixture model".into(),
            resolved_model: Some("fixture-model".into()),
            effort_levels: vec!["low".into(), "high".into()],
            default_effort: Some("low".into()),
        }])
    }
    async fn open_project(
        &self,
        environment: &Environment,
        project: Option<&Project>,
        previous: Option<&str>,
        model: &str,
        effort: Option<&str>,
        input: &str,
    ) -> Result<ExecutionSession, ExecutionError> {
        let Some(project) = project else {
            return self.open(environment, previous, model, effort, input).await;
        };
        if self.project_missing.load(Ordering::SeqCst) {
            return Err(ExecutionError::PreflightUnavailable);
        }
        self.project_opens.lock().unwrap().push((
            project.id,
            format!("/home/factory/Documents/{}", project.directory),
            previous.map(str::to_owned),
        ));
        let (tx, rx) = mpsc::channel(4);
        tx.send(ExecutionEvent::ThreadReady {
            provider_thread: "fixture-project-thread".into(),
        })
        .await
        .unwrap();
        tx.send(ExecutionEvent::Completed).await.unwrap();
        Ok(ExecutionSession {
            control: Box::new(Control {
                started: self.started.clone(),
                stopped: self.stopped.clone(),
            }),
            output: Box::new(Output(rx)),
        })
    }
    async fn open(
        &self,
        _: &Environment,
        _: Option<&str>,
        _: &str,
        effort: Option<&str>,
        _: &str,
    ) -> Result<ExecutionSession, ExecutionError> {
        *self.effort.lock().unwrap() = effort.map(str::to_owned);
        self.opens.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = *self.fail_open.lock().unwrap() {
            return Err(error);
        }
        let rx = self
            .rx
            .lock()
            .await
            .take()
            .ok_or(ExecutionError::Conflict)?;
        Ok(ExecutionSession {
            control: Box::new(Control {
                started: self.started.clone(),
                stopped: self.stopped.clone(),
            }),
            output: Box::new(Output(rx)),
        })
    }
}

struct ConversationFixture {
    app: Router,
    runtime: Arc<factory_api::conversations::runtime::ConversationRuntime>,
    auth: AuthState,
    store: Arc<Store>,
    data: Arc<Conversations>,
    provider: Arc<Provider>,
    other_provider: Option<Arc<Provider>>,
    environments: Arc<EnvironmentService>,
    conversation_service: Arc<ConversationService>,
    tokens: [String; 2],
    hashes: [CredentialHash; 2],
}
impl ConversationFixture {
    async fn new() -> Self {
        Self::create(false).await
    }
    async fn create(with_second_provider: bool) -> Self {
        Self::with_connections(with_second_provider, true, true).await
    }
    async fn with_connections(with_second_provider: bool, codex: bool, claude: bool) -> Self {
        let tokens = [
            URL_SAFE_NO_PAD.encode([7u8; 32]),
            URL_SAFE_NO_PAD.encode([8u8; 32]),
        ];
        let hashes = tokens
            .each_ref()
            .map(|s| CredentialHash(Sha256::digest(s.as_bytes()).into()));
        let users = [Uuid::new_v4(), Uuid::new_v4()];
        let sessions = hashes
            .iter()
            .zip(users)
            .map(|(hash, id)| {
                (
                    *hash,
                    ActiveSession {
                        user: User {
                            id,
                            display_name: "Fixture".into(),
                            handle: "fixture".into(),
                            provider: "github".into(),
                            role: Role::Member,
                            status: AccessStatus::Active,
                            version: 1,
                            access_enabled_at: Some(Utc::now()),
                        },
                        csrf_token: CSRF.into(),
                        expires_at: Utc::now() + ChronoDuration::hours(1),
                    },
                )
            })
            .collect();
        let config = EnvironmentStart {
            installation_id: Uuid::new_v4(),
            image_id: "fixture-image".into(),
            policy_version: "2".into(),
        };
        let availability = EnvironmentAvailability::default();
        let store = Arc::new(Store {
            sessions: Mutex::new(sessions),
            environment_reads: AtomicUsize::new(0),
            environment: Mutex::new(Environment {
                id: Uuid::new_v4(),
                user_id: users[0],
                installation_id: config.installation_id,
                desired_state: DesiredState::Running,
                observed_state: ObservedState::Running,
                generation: 1,
                provisioning_state: ProvisioningState::VolumeReady,
                volume_name: "fixture-volume".into(),
                container_name: "fixture-container".into(),
                image_id: config.image_id.clone(),
                policy_version: config.policy_version.clone(),
                observed_at: Some(Utc::now()),
                error_code: None,
                next_reconcile_at: Utc::now(),
            }),
        });
        EnvironmentReconciler::new(
            store.clone(),
            Arc::new(NoEnvironmentRuntime),
            Box::new(Supervisor),
            config.clone(),
            availability.clone(),
        )
        .unwrap()
        .next_batch()
        .await
        .unwrap();
        let env = Arc::new(EnvironmentService::new(store.clone(), config, availability).unwrap());
        let accounts = Arc::new(AgentConnectionService::new(
            env.clone(),
            Arc::new(Connected(codex)),
        ));
        let data = Arc::new(Conversations::default());
        let other_accounts = Arc::new(AgentConnectionService::new(
            env.clone(),
            Arc::new(Connected(claude)),
        ));
        let projects = Arc::new(ProjectService::new(env.clone(), data.clone()));
        let service = Arc::new(ConversationService::new_with_projects(
            env.clone(),
            data.clone(),
            projects,
        ));
        let (tx, rx) = mpsc::channel(32);
        let provider = Arc::new(Provider {
            tx,
            rx: AsyncMutex::new(Some(rx)),
            opens: AtomicUsize::new(0),
            effort: Mutex::new(None),
            fail_open: Mutex::new(None),
            model_started: AtomicUsize::new(0),
            fail_models: Mutex::new(None),
            hold_models: AtomicBool::new(false),
            release_models: Notify::new(),
            started: Arc::new(AtomicUsize::new(0)),
            stopped: Arc::new(AtomicUsize::new(0)),
            project_opens: Mutex::new(Vec::new()),
            project_missing: AtomicBool::new(false),
        });
        let other_provider = if with_second_provider {
            let (tx, rx) = mpsc::channel(32);
            Some(Arc::new(Provider {
                tx,
                rx: AsyncMutex::new(Some(rx)),
                opens: AtomicUsize::new(0),
                effort: Mutex::new(None),
                fail_open: Mutex::new(None),
                model_started: AtomicUsize::new(0),
                fail_models: Mutex::new(None),
                hold_models: AtomicBool::new(false),
                release_models: Notify::new(),
                started: Arc::new(AtomicUsize::new(0)),
                stopped: Arc::new(AtomicUsize::new(0)),
                project_opens: Mutex::new(Vec::new()),
                project_missing: AtomicBool::new(false),
            }))
        } else {
            None
        };
        let runtime = if let Some(other) = &other_provider {
            factory_api::conversations::runtime::with_providers(
                service.clone(),
                vec![
                    ExecutionProvider {
                        id: "codex",
                        display_name: "Codex",
                        accounts,
                        runtime: provider.clone(),
                    },
                    ExecutionProvider {
                        id: "claude",
                        display_name: "Claude",
                        accounts: other_accounts,
                        runtime: other.clone(),
                    },
                ],
            )
            .unwrap()
        } else {
            factory_api::conversations::runtime::new(service.clone(), provider.clone(), accounts)
        };
        let auth = AuthState::new(
            Arc::new(IdentityService::new(store.clone(), Arc::new(NoProvider))),
            &RuntimeConfig::from_values(|_| None).unwrap(),
        );
        Self {
            app: conversations::router(auth.clone(), Some(runtime.clone())),
            runtime,
            auth,
            store,
            data,
            provider,
            other_provider,
            environments: env,
            conversation_service: service,
            tokens,
            hashes,
        }
    }
    async fn call(
        &self,
        method: &str,
        path: &str,
        user: usize,
        body: &str,
    ) -> axum::response::Response {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("cookie", format!("factory_session={}", self.tokens[user]))
            .header("origin", ORIGIN)
            .header("x-csrf-token", CSRF)
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap();
        self.app.clone().oneshot(req).await.unwrap()
    }
}
async fn body_json(r: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(r.into_body(), 16384).await.unwrap()).unwrap()
}
async fn until(f: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !f() {
            tokio::time::sleep(Duration::from_millis(5)).await
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn owned_conversation_streams_and_persists_without_leaking_provider_references() {
    use futures_util::StreamExt;
    let f = ConversationFixture::new().await;
    let create = f.call("POST", "/api/conversations", 0, "").await;
    assert_eq!(create.status(), StatusCode::OK);
    let id = body_json(create).await["id"].as_str().unwrap().to_string();
    assert_eq!(
        f.call("GET", &format!("/api/conversations/{id}"), 1, "")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let operation = Uuid::new_v4();
    let input =
        json!({"operationId":operation,"model":"fixture-model","input":"hello"}).to_string();
    let path = format!("/api/conversations/{id}/runs");
    let first = f.call("POST", &path, 0, &input).await;
    assert_eq!(first.status(), StatusCode::OK);
    let run = body_json(first).await["id"].as_str().unwrap().to_string();
    let replay = f.call("POST", &path, 0, &input).await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(body_json(replay).await["id"], run);
    f.provider
        .tx
        .send(ExecutionEvent::ThreadReady {
            provider_thread: "private-provider-thread".into(),
        })
        .await
        .unwrap();
    until(|| f.provider.started.load(Ordering::SeqCst) == 1).await;
    assert_eq!(f.provider.opens.load(Ordering::SeqCst), 1);
    let event_path = format!("/api/conversations/{id}/runs/{run}/events");
    assert_ne!(
        f.call("GET", &event_path, 1, "").await.status(),
        StatusCode::OK
    );
    let stream = f.call("GET", &event_path, 0, "").await;
    assert_eq!(stream.status(), StatusCode::OK);
    let shared_origin_config = RuntimeConfig::from_values(|key| match key {
        "FACTORY_API_BIND" => Some("127.0.0.1:3000".into()),
        "FACTORY_API_ORIGIN" => Some(ORIGIN.into()),
        _ => None,
    })
    .unwrap();
    let shared_origin_auth = AuthState::new(
        Arc::new(IdentityService::new(f.store.clone(), Arc::new(NoProvider))),
        &shared_origin_config,
    );
    let shared_origin_app = conversations::router(shared_origin_auth, Some(f.runtime.clone()));
    let request = || {
        Request::builder()
            .uri(&event_path)
            .header("cookie", format!("factory_session={}", f.tokens[0]))
    };
    let browser_request = || {
        request()
            .header("sec-fetch-site", "same-origin")
            .body(Body::empty())
            .unwrap()
    };
    let same_origin_stream = shared_origin_app
        .clone()
        .oneshot(browser_request())
        .await
        .unwrap();
    assert_eq!(same_origin_stream.status(), StatusCode::OK);

    assert_eq!(
        f.app
            .clone()
            .oneshot(browser_request())
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN,
        "a split deployment still requires the configured frontend Origin"
    );
    for rejected in [
        request(),
        request().header("sec-fetch-site", "same-site"),
        request().header("sec-fetch-site", "cross-site"),
        request()
            .header("sec-fetch-site", "same-origin")
            .header("sec-fetch-site", "cross-site"),
        request()
            .header("sec-fetch-site", "same-origin")
            .header("origin", "null"),
        request()
            .header("sec-fetch-site", "same-origin")
            .header("origin", ORIGIN)
            .header("origin", ORIGIN),
    ] {
        let response = shared_origin_app
            .clone()
            .oneshot(rejected.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let no_session = Request::builder()
        .uri(&event_path)
        .header("sec-fetch-site", "same-origin")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        shared_origin_app
            .oneshot(no_session)
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let mut chunks = same_origin_stream.into_body().into_data_stream();
    f.provider
        .tx
        .send(ExecutionEvent::Delta {
            provider_item: "private-item".into(),
            text: "visible text".into(),
        })
        .await
        .unwrap();
    let chunk = tokio::time::timeout(Duration::from_secs(3), chunks.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let payload = String::from_utf8(chunk.to_vec()).unwrap();
    assert!(payload.contains("visible text"));
    assert!(!payload.contains("private-item"));
    assert!(!payload.contains("private-provider-thread"));
    f.provider
        .tx
        .send(ExecutionEvent::Message {
            provider_item: "private-item".into(),
            text: "complete answer".into(),
        })
        .await
        .unwrap();
    f.provider.tx.send(ExecutionEvent::Completed).await.unwrap();
    until(|| {
        f.data
            .0
            .lock()
            .unwrap()
            .run
            .as_ref()
            .is_some_and(|r| r.status == RunStatus::Completed)
    })
    .await;
    let history = body_json(
        f.call("GET", &format!("/api/conversations/{id}"), 0, "")
            .await,
    )
    .await
    .to_string();
    assert!(history.contains("complete answer"));
    assert!(!history.contains("private-provider-thread"));
    assert!(!history.contains("private-item"));
}
#[tokio::test]
async fn revoked_session_closes_stream_and_requests_interruption() {
    use futures_util::StreamExt;
    let f = ConversationFixture::new().await;
    let id = body_json(f.call("POST", "/api/conversations", 0, "").await).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let input =
        json!({"operationId":Uuid::new_v4(),"model":"fixture-model","input":"hello"}).to_string();
    let run = body_json(
        f.call("POST", &format!("/api/conversations/{id}/runs"), 0, &input)
            .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    f.provider
        .tx
        .send(ExecutionEvent::ThreadReady {
            provider_thread: "private-provider-thread".into(),
        })
        .await
        .unwrap();
    until(|| f.provider.started.load(Ordering::SeqCst) == 1).await;
    let r = f
        .call(
            "GET",
            &format!("/api/conversations/{id}/runs/{run}/events"),
            0,
            "",
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    let mut chunks = r.into_body().into_data_stream();
    f.auth.service().logout(f.hashes[0]).await.unwrap();
    f.provider
        .tx
        .send(ExecutionEvent::Delta {
            provider_item: "private".into(),
            text: "must not be delivered".into(),
        })
        .await
        .unwrap();
    let next = tokio::time::timeout(Duration::from_secs(3), chunks.next())
        .await
        .unwrap();
    assert!(
        next.is_none(),
        "revoked stream must close before forwarding"
    );
    until(|| f.provider.stopped.load(Ordering::SeqCst) > 0).await;
}

#[tokio::test]
async fn conversation_routes_require_session_origin_and_csrf() {
    let f = ConversationFixture::new().await;
    let no_session = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/conversations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_session.status(), StatusCode::UNAUTHORIZED);
    for (origin, csrf) in [
        ("https://attacker.example", CSRF),
        (ORIGIN, "incorrect"),
        ("", CSRF),
    ] {
        let response = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/conversations")
                    .header("cookie", format!("factory_session={}", f.tokens[0]))
                    .header("origin", origin)
                    .header("x-csrf-token", csrf)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    assert!(f.data.0.lock().unwrap().conversation.is_none());
}

#[tokio::test]
async fn recoverable_runs_are_private_and_require_a_fresh_stopped_environment() {
    let f = ConversationFixture::new().await;
    let absent = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/conversations/recoverable")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(absent.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        f.call("GET", "/api/conversations/recoverable", 0, "")
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let id = body_json(f.call("POST", "/api/conversations", 0, "").await).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let input =
        json!({"operationId":Uuid::new_v4(),"model":"fixture-model","input":"hello"}).to_string();
    let run = body_json(
        f.call("POST", &format!("/api/conversations/{id}/runs"), 0, &input)
            .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    {
        let mut env = f.store.environment.lock().unwrap();
        env.desired_state = DesiredState::Stopped;
        env.observed_state = ObservedState::Stopped;
        env.generation += 1;
        env.observed_at = Some(Utc::now());
    }
    let response = f.call("GET", "/api/conversations/recoverable", 0, "").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["conversationId"], id);
    assert_eq!(body[0]["runId"], run);
    let other = f.call("GET", "/api/conversations/recoverable", 1, "").await;
    assert_eq!(other.status(), StatusCode::OK);
    assert_eq!(body_json(other).await, json!([]));
    f.store.sessions.lock().unwrap().remove(&f.hashes[0]);
    assert_eq!(
        f.call("GET", "/api/conversations/recoverable", 0, "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn preflight_failure_is_failed_but_post_start_uncertainty_stays_blocking() {
    for (failure, expected) in [
        (ExecutionError::PreflightUnavailable, RunStatus::Failed),
        (ExecutionError::CleanupUncertain, RunStatus::Uncertain),
        (ExecutionError::LaunchUncertain, RunStatus::Uncertain),
    ] {
        let f = ConversationFixture::new().await;
        *f.provider.fail_open.lock().unwrap() = Some(failure);
        let id = body_json(f.call("POST", "/api/conversations", 0, "").await).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let input = json!({"operationId":Uuid::new_v4(),"model":"fixture-model","input":"hello"})
            .to_string();
        assert_eq!(
            f.call("POST", &format!("/api/conversations/{id}/runs"), 0, &input)
                .await
                .status(),
            StatusCode::OK
        );
        until(|| {
            f.data
                .0
                .lock()
                .unwrap()
                .run
                .as_ref()
                .is_some_and(|r| r.status == expected)
        })
        .await;
        let response = body_json(
            f.call("GET", &format!("/api/conversations/{id}"), 0, "")
                .await,
        )
        .await;
        assert_eq!(
            response["runs"][0]["status"],
            if expected == RunStatus::Failed {
                "failed"
            } else {
                "uncertain"
            }
        );
        assert_eq!(f.provider.started.load(Ordering::SeqCst), 0);
        assert_eq!(
            f.data.0.lock().unwrap().run_settled,
            expected == RunStatus::Failed
        );
    }
}

#[tokio::test]
async fn stream_burst_does_not_requery_full_history_per_delta() {
    use futures_util::StreamExt;
    let f = ConversationFixture::new().await;
    let id = body_json(f.call("POST", "/api/conversations", 0, "").await).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    let input =
        json!({"operationId":Uuid::new_v4(),"model":"fixture-model","input":"hello"}).to_string();
    let run = body_json(
        f.call("POST", &format!("/api/conversations/{id}/runs"), 0, &input)
            .await,
    )
    .await["id"]
        .as_str()
        .unwrap()
        .to_string();
    f.provider
        .tx
        .send(ExecutionEvent::ThreadReady {
            provider_thread: "private-provider-thread".into(),
        })
        .await
        .unwrap();
    until(|| f.provider.started.load(Ordering::SeqCst) == 1).await;
    let response = f
        .call(
            "GET",
            &format!("/api/conversations/{id}/runs/{run}/events"),
            0,
            "",
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut chunks = response.into_body().into_data_stream();
    let before = f.data.1.load(Ordering::SeqCst);
    for _ in 0..16 {
        f.provider
            .tx
            .send(ExecutionEvent::Delta {
                provider_item: "private-item".into(),
                text: "delta".into(),
            })
            .await
            .unwrap();
        let chunk = tokio::time::timeout(Duration::from_secs(3), chunks.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&chunk).contains("delta"));
    }
    let checks = f.data.1.load(Ordering::SeqCst) - before;
    assert!(checks <= 3, "burst triggered {checks} full history reads");
}

#[tokio::test]
async fn account_changes_and_logout_refuse_open_or_uncertain_owned_runs() {
    for uncertain in [false, true] {
        let f = ConversationFixture::new().await;
        if uncertain {
            *f.provider.fail_open.lock().unwrap() = Some(ExecutionError::CleanupUncertain);
        }
        let actor = {
            let sessions = f.store.sessions.lock().unwrap();
            EnvironmentActor {
                user_id: sessions[&f.hashes[0]].user.id,
                session_hash: f.hashes[0],
            }
        };
        let other = {
            let sessions = f.store.sessions.lock().unwrap();
            EnvironmentActor {
                user_id: sessions[&f.hashes[1]].user.id,
                session_hash: f.hashes[1],
            }
        };
        let id = body_json(f.call("POST", "/api/conversations", 0, "").await).await["id"]
            .as_str()
            .unwrap()
            .to_string();
        let input = json!({"operationId":Uuid::new_v4(),"model":"fixture-model","input":"hello"})
            .to_string();
        assert_eq!(
            f.call("POST", &format!("/api/conversations/{id}/runs"), 0, &input)
                .await
                .status(),
            StatusCode::OK
        );
        if uncertain {
            until(|| {
                f.data
                    .0
                    .lock()
                    .unwrap()
                    .run
                    .as_ref()
                    .is_some_and(|r| r.status == RunStatus::Uncertain)
            })
            .await;
        }
        assert!(matches!(
            f.runtime.acquire_account_change(actor).await,
            Err(AgentConnectionError::Conflict)
        ));
        assert!(matches!(
            f.runtime.logout(actor).await,
            Err(AgentConnectionError::Conflict)
        ));
        let other_guard = f.runtime.acquire_account_change(other).await;
        assert!(
            other_guard.is_ok(),
            "another user must not inherit this run's guard"
        );
    }
}

#[tokio::test]
async fn overlapping_model_requests_share_one_probe_and_release_the_slot() {
    let f = Arc::new(ConversationFixture::new().await);
    f.provider.hold_models.store(true, Ordering::SeqCst);
    let first_fixture = f.clone();
    let first = tokio::spawn(async move {
        first_fixture
            .call("GET", "/api/conversations/models", 0, "")
            .await
    });
    until(|| f.provider.model_started.load(Ordering::SeqCst) == 1).await;
    let reads = f.store.environment_reads.load(Ordering::SeqCst);
    let second_fixture = f.clone();
    let second = tokio::spawn(async move {
        second_fixture
            .call("GET", "/api/conversations/models", 0, "")
            .await
    });
    until(|| f.store.environment_reads.load(Ordering::SeqCst) > reads).await;
    assert!(!first.is_finished());
    assert!(!second.is_finished());
    assert_eq!(f.provider.model_started.load(Ordering::SeqCst), 1);
    f.provider.hold_models.store(false, Ordering::SeqCst);
    f.provider.release_models.notify_one();
    let (first, second) = tokio::time::timeout(Duration::from_secs(2), async {
        (first.await.unwrap(), second.await.unwrap())
    })
    .await
    .expect("shared model requests must finish after provider release");
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(body_json(first).await, body_json(second).await);
    assert!(f.data.0.lock().unwrap().probes.is_empty());
    assert_eq!(
        f.call("GET", "/api/conversations/models", 0, "")
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(f.provider.model_started.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn shared_model_requests_reauthorize_each_session_before_returning_models() {
    for revoked in [0, 1] {
        let f = Arc::new(ConversationFixture::new().await);
        {
            let mut sessions = f.store.sessions.lock().unwrap();
            let user = sessions[&f.hashes[0]].user.clone();
            sessions.get_mut(&f.hashes[1]).unwrap().user = user;
        }
        f.provider.hold_models.store(true, Ordering::SeqCst);
        let first_fixture = f.clone();
        let first = tokio::spawn(async move {
            first_fixture
                .call("GET", "/api/conversations/models", 0, "")
                .await
        });
        until(|| f.provider.model_started.load(Ordering::SeqCst) == 1).await;
        let reads = f.store.environment_reads.load(Ordering::SeqCst);
        let second_fixture = f.clone();
        let second = tokio::spawn(async move {
            second_fixture
                .call("GET", "/api/conversations/models", 1, "")
                .await
        });
        until(|| f.store.environment_reads.load(Ordering::SeqCst) > reads).await;
        f.store.sessions.lock().unwrap().remove(&f.hashes[revoked]);
        f.provider.hold_models.store(false, Ordering::SeqCst);
        f.provider.release_models.notify_one();
        let responses = tokio::time::timeout(Duration::from_secs(2), async {
            [first.await.unwrap(), second.await.unwrap()]
        })
        .await
        .expect("each session must finish its own authorization check");
        for (index, response) in responses.into_iter().enumerate() {
            assert_eq!(
                response.status(),
                if index == revoked {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::OK
                }
            );
            if index == revoked {
                assert!(
                    !body_json(response)
                        .await
                        .to_string()
                        .contains("fixture-model")
                );
            }
        }
        assert_eq!(f.provider.model_started.load(Ordering::SeqCst), 1);
        assert!(f.data.0.lock().unwrap().probes.is_empty());
    }
}

#[tokio::test]
async fn dropping_the_initial_http_request_does_not_restart_or_abandon_the_shared_probe() {
    let f = Arc::new(ConversationFixture::new().await);
    f.provider.hold_models.store(true, Ordering::SeqCst);
    let first_fixture = f.clone();
    let first = tokio::spawn(async move {
        first_fixture
            .call("GET", "/api/conversations/models", 0, "")
            .await
    });
    until(|| f.provider.model_started.load(Ordering::SeqCst) == 1).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let reads = f.store.environment_reads.load(Ordering::SeqCst);
    let second_fixture = f.clone();
    let second = tokio::spawn(async move {
        second_fixture
            .call("GET", "/api/conversations/models", 0, "")
            .await
    });
    until(|| f.store.environment_reads.load(Ordering::SeqCst) > reads).await;
    assert_eq!(f.provider.model_started.load(Ordering::SeqCst), 1);
    assert_eq!(f.data.0.lock().unwrap().probes.len(), 1);
    f.provider.hold_models.store(false, Ordering::SeqCst);
    f.provider.release_models.notify_one();
    let response = tokio::time::timeout(Duration::from_secs(2), second)
        .await
        .expect("the surviving request must finish after provider release")
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(f.data.0.lock().unwrap().probes.is_empty());
}

#[tokio::test]
async fn model_probes_allow_independent_users() {
    let f = ConversationFixture::new().await;
    f.provider.hold_models.store(true, Ordering::SeqCst);
    let mut actors = Vec::new();
    for n in 20u8..29 {
        let hash = CredentialHash([n; 32]);
        let user = User {
            id: Uuid::new_v4(),
            display_name: "Fixture".into(),
            handle: "fixture".into(),
            provider: "github".into(),
            role: Role::Member,
            status: AccessStatus::Active,
            version: 1,
            access_enabled_at: Some(Utc::now()),
        };
        actors.push(EnvironmentActor {
            user_id: user.id,
            session_hash: hash,
        });
        f.store.sessions.lock().unwrap().insert(
            hash,
            ActiveSession {
                user,
                csrf_token: CSRF.into(),
                expires_at: Utc::now() + ChronoDuration::hours(1),
            },
        );
    }
    let mut tasks = Vec::new();
    for actor in actors.iter().copied() {
        let runtime = f.runtime.clone();
        tasks.push(tokio::spawn(async move { runtime.models(actor).await }));
    }
    until(|| f.provider.model_started.load(Ordering::SeqCst) == 9).await;
    f.provider.hold_models.store(false, Ordering::SeqCst);
    f.provider.release_models.notify_waiters();
    for task in tasks {
        assert!(task.await.unwrap().is_ok());
    }
}

#[tokio::test]
async fn model_probe_recovery_requires_authenticated_stopped_owner() {
    let f = ConversationFixture::new().await;
    let lease = Uuid::new_v4();
    let env = f.store.environment.lock().unwrap().clone();
    f.data
        .0
        .lock()
        .unwrap()
        .probes
        .insert(lease, (env.user_id, env.id, env.generation, false));
    let path = "/api/conversations/model-probe/recover";
    let no_session = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_session.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        f.call("POST", path, 0, "").await.status(),
        StatusCode::CONFLICT
    );
    {
        let mut current = f.store.environment.lock().unwrap();
        current.desired_state = DesiredState::Stopped;
        current.observed_state = ObservedState::Stopped;
        current.generation += 1;
        current.observed_at = Some(Utc::now());
    }
    assert_eq!(
        f.call("POST", path, 1, "").await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(f.data.0.lock().unwrap().probes.contains_key(&lease));
    assert_eq!(
        f.call("POST", path, 0, "").await.status(),
        StatusCode::CONFLICT
    );
    assert!(f.data.0.lock().unwrap().probes.contains_key(&lease));
    f.data
        .settle_model_probe_launch(lease, env.id, env.generation)
        .await
        .unwrap();
    assert_eq!(
        f.call("POST", path, 0, "").await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(!f.data.0.lock().unwrap().probes.contains_key(&lease));
    assert_eq!(
        f.call("POST", path, 0, "").await.status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn account_coordination_is_scoped_to_user_and_serializes_same_owner() {
    let f = ConversationFixture::new().await;
    let actors = {
        let sessions = f.store.sessions.lock().unwrap();
        [0, 1].map(|i| EnvironmentActor {
            user_id: sessions[&f.hashes[i]].user.id,
            session_hash: f.hashes[i],
        })
    };
    let held = f.runtime.acquire_account_change(actors[0]).await.unwrap();
    let other = tokio::time::timeout(
        Duration::from_secs(1),
        f.runtime.acquire_account_change(actors[1]),
    )
    .await
    .expect("different owner must not wait")
    .unwrap();
    drop(other);
    let mut same = Box::pin(f.runtime.acquire_account_change(actors[0]));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut same)
            .await
            .is_err()
    );
    drop(held);
    let next = tokio::time::timeout(Duration::from_secs(1), same)
        .await
        .expect("same owner should proceed after release")
        .unwrap();
    drop(next);
}

#[tokio::test]
async fn paused_provider_launch_keeps_lease_during_stop_and_restart() {
    let f = ConversationFixture::new().await;
    f.provider.hold_models.store(true, Ordering::SeqCst);
    let actor = {
        let sessions = f.store.sessions.lock().unwrap();
        EnvironmentActor {
            user_id: sessions[&f.hashes[0]].user.id,
            session_hash: f.hashes[0],
        }
    };
    let rt = f.runtime.clone();
    let launch = tokio::spawn(async move { rt.models(actor).await });
    until(|| f.provider.model_started.load(Ordering::SeqCst) == 1).await;
    assert_eq!(f.data.0.lock().unwrap().probes.len(), 1);
    {
        let mut env = f.store.environment.lock().unwrap();
        env.desired_state = DesiredState::Stopped;
        env.observed_state = ObservedState::Stopped;
        env.generation = 2;
        env.observed_at = Some(Utc::now());
    }
    assert!(matches!(
        f.runtime.service.recover_model_probe_stopped(actor).await,
        Err(ConversationError::Conflict)
    ));
    assert_eq!(
        f.data.0.lock().unwrap().probes.len(),
        1,
        "paused start must keep its lease"
    );
    {
        let mut env = f.store.environment.lock().unwrap();
        env.desired_state = DesiredState::Running;
        env.observed_state = ObservedState::Running;
        env.generation = 3;
        env.observed_at = Some(Utc::now());
    }
    assert_eq!(
        f.data.0.lock().unwrap().probes.len(),
        1,
        "restart cannot reuse the slot"
    );
    f.provider.hold_models.store(false, Ordering::SeqCst);
    f.provider.release_models.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(2), launch)
        .await
        .expect("the completed probe must release its reservation")
        .unwrap();
    assert_eq!(result, Err(ConversationError::Conflict));
    assert!(
        f.data.0.lock().unwrap().probes.is_empty(),
        "a conclusive completed process releases its lease"
    );
}

#[tokio::test]
async fn model_probe_preflight_releases_capacity_but_submitted_timeout_keeps_it() {
    for (failure, retained) in [
        (ExecutionError::PreflightUnavailable, false),
        (ExecutionError::LaunchUncertain, true),
    ] {
        let f = ConversationFixture::new().await;
        *f.provider.fail_models.lock().unwrap() = Some(failure);
        let actor = {
            let sessions = f.store.sessions.lock().unwrap();
            EnvironmentActor {
                user_id: sessions[&f.hashes[0]].user.id,
                session_hash: f.hashes[0],
            }
        };
        assert!(matches!(
            f.runtime.models(actor).await,
            Err(ConversationError::Unavailable)
        ));
        assert_eq!(!f.data.0.lock().unwrap().probes.is_empty(), retained);
        if retained {
            let probe = *f.data.0.lock().unwrap().probes.values().next().unwrap();
            assert!(
                !probe.3,
                "a submitted launch without a result must stay unsettled"
            );
        } else {
            *f.provider.fail_models.lock().unwrap() = None;
            assert!(f.runtime.models(actor).await.is_ok());
        }
    }
}

#[tokio::test]
async fn models_and_turns_are_dispatched_to_the_selected_connected_provider() {
    let f = ConversationFixture::create(true).await;
    let models = body_json(f.call("GET", "/api/conversations/models", 0, "").await).await;
    assert_eq!(models.as_array().unwrap().len(), 2);
    assert_eq!(models[0]["id"], "factory:codex:fixture-model");
    assert_eq!(models[0]["name"], "Codex · Fixture model");
    assert_eq!(models[1]["id"], "factory:claude:fixture-model");
    assert_eq!(models[1]["name"], "Claude · Fixture model");
    assert_eq!(models[1]["resolvedModel"], "fixture-model");
    assert_eq!(models[1]["effortLevels"], json!(["low", "high"]));
    let create = f.call("POST", "/api/conversations", 0, "").await;
    let id = body_json(create).await["id"].as_str().unwrap().to_owned();
    let invalid = json!({"operationId":Uuid::new_v4(),"model":"factory:claude:fixture-model","effort":"max","input":"hello"}).to_string();
    let rejected = f
        .call(
            "POST",
            &format!("/api/conversations/{id}/runs"),
            0,
            &invalid,
        )
        .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        f.other_provider
            .as_ref()
            .unwrap()
            .opens
            .load(Ordering::SeqCst),
        0
    );
    let operation = Uuid::new_v4();
    let body = json!({"operationId":operation,"model":"factory:claude:fixture-model","effort":"high","input":"hello"}).to_string();
    let response = f
        .call("POST", &format!("/api/conversations/{id}/runs"), 0, &body)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let run = body_json(response).await;
    assert_eq!(run["model"], "factory:claude:fixture-model");
    assert_eq!(run["effort"], "high");
    let other = f.other_provider.as_ref().unwrap();
    until(|| other.opens.load(Ordering::SeqCst) == 1).await;
    assert_eq!(other.effort.lock().unwrap().as_deref(), Some("high"));
    assert_eq!(f.provider.opens.load(Ordering::SeqCst), 0);
    other
        .tx
        .send(ExecutionEvent::ThreadReady {
            provider_thread: "claude-private-thread".into(),
        })
        .await
        .unwrap();
    until(|| other.started.load(Ordering::SeqCst) == 1).await;
    other.tx.send(ExecutionEvent::Completed).await.unwrap();
    until(|| {
        f.data
            .0
            .lock()
            .unwrap()
            .run
            .as_ref()
            .is_some_and(|run| run.status == RunStatus::Completed)
    })
    .await;
    *other.fail_models.lock().unwrap() = Some(ExecutionError::PreflightUnavailable);
    let discovery_count = other.model_started.load(Ordering::SeqCst);
    let replay = f
        .call("POST", &format!("/api/conversations/{id}/runs"), 0, &body)
        .await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(body_json(replay).await["id"], run["id"]);
    assert_eq!(other.model_started.load(Ordering::SeqCst), discovery_count);
    assert_eq!(other.opens.load(Ordering::SeqCst), 1);
    let changed_input = json!({"operationId":operation,"model":"factory:claude:fixture-model","effort":"high","input":"different"}).to_string();
    assert_eq!(
        f.call(
            "POST",
            &format!("/api/conversations/{id}/runs"),
            0,
            &changed_input
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let changed = json!({"operationId":operation,"model":"factory:claude:fixture-model","effort":"low","input":"hello"}).to_string();
    let replay = f
        .call(
            "POST",
            &format!("/api/conversations/{id}/runs"),
            0,
            &changed,
        )
        .await;
    assert_eq!(replay.status(), StatusCode::CONFLICT);
    let history = body_json(
        f.call("GET", &format!("/api/conversations/{id}"), 0, "")
            .await,
    )
    .await
    .to_string();
    assert!(!history.contains("claude-private-thread"));
}

#[tokio::test]
async fn model_discovery_isolates_confirmed_provider_failures() {
    for failed_claude in [false, true] {
        let f = ConversationFixture::create(true).await;
        let other = f.other_provider.as_ref().unwrap();
        let failed = if failed_claude { other } else { &f.provider };
        *failed.fail_models.lock().unwrap() = Some(ExecutionError::PreflightUnavailable);
        let response = f.call("GET", "/api/conversations/models", 0, "").await;
        assert_eq!(response.status(), StatusCode::OK);
        let models = body_json(response).await;
        let expected = if failed_claude {
            "factory:codex:fixture-model"
        } else {
            "factory:claude:fixture-model"
        };
        assert_eq!(models.as_array().unwrap().len(), 1);
        assert_eq!(models[0]["id"], expected);
        assert!(f.data.0.lock().unwrap().probes.is_empty());
        let create = f.call("POST", "/api/conversations", 0, "").await;
        let id = body_json(create).await["id"].as_str().unwrap().to_owned();
        let body =
            json!({"operationId":Uuid::new_v4(),"model":expected,"effort":"high","input":"hello"})
                .to_string();
        assert_eq!(
            f.call("POST", &format!("/api/conversations/{id}/runs"), 0, &body)
                .await
                .status(),
            StatusCode::OK
        );
    }
    for error in [
        ExecutionError::LaunchUncertain,
        ExecutionError::CleanupUncertain,
    ] {
        let f = ConversationFixture::create(true).await;
        *f.other_provider
            .as_ref()
            .unwrap()
            .fail_models
            .lock()
            .unwrap() = Some(error);
        assert_eq!(
            f.call("GET", "/api/conversations/models", 0, "")
                .await
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        let probes = f.data.0.lock().unwrap();
        assert_eq!(probes.probes.len(), 1);
        assert_eq!(
            probes.probes.values().next().unwrap().3,
            error == ExecutionError::CleanupUncertain
        );
    }
}

#[tokio::test]
async fn models_require_at_least_one_connection_and_include_only_connected_providers() {
    let only_claude = ConversationFixture::with_connections(true, false, true).await;
    let response = only_claude
        .call("GET", "/api/conversations/models", 0, "")
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let models = body_json(response).await;
    assert_eq!(models.as_array().unwrap().len(), 1);
    assert_eq!(models[0]["id"], "factory:claude:fixture-model");
    let created = only_claude.call("POST", "/api/conversations", 0, "").await;
    let id = body_json(created).await["id"].as_str().unwrap().to_owned();
    let body = json!({"operationId":Uuid::new_v4(),"model":"factory:claude:fixture-model","input":"hello"}).to_string();
    let response = only_claude
        .call("POST", &format!("/api/conversations/{id}/runs"), 0, &body)
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    let other = only_claude.other_provider.as_ref().unwrap();
    until(|| other.opens.load(Ordering::SeqCst) == 1).await;
    assert_eq!(only_claude.provider.opens.load(Ordering::SeqCst), 0);

    let none = ConversationFixture::with_connections(true, false, false).await;
    let response = none.call("GET", "/api/conversations/models", 0, "").await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[derive(Default)]
struct ProjectTerminal {
    attached: Mutex<Vec<(Uuid, Option<Uuid>, String)>>,
    detached: Mutex<Vec<Uuid>>,
}
struct ProjectTerminalInput;
struct ProjectTerminalOutput;
#[async_trait]
impl TerminalInput for ProjectTerminalInput {
    async fn write(&mut self, _: &[u8]) -> Result<(), TerminalError> {
        Ok(())
    }
}
#[async_trait]
impl TerminalOutput for ProjectTerminalOutput {
    async fn read(&mut self) -> Result<Option<Vec<u8>>, TerminalError> {
        Ok(None)
    }
}
#[async_trait]
impl TerminalRuntime for ProjectTerminal {
    async fn attach(&self, _: &Environment) -> Result<TerminalProcess, TerminalError> {
        unreachable!()
    }
    async fn attach_conversation(
        &self,
        _: &Environment,
        conversation: Uuid,
        project: Option<&Project>,
    ) -> Result<TerminalProcess, TerminalError> {
        self.attached.lock().unwrap().push((
            conversation,
            project.map(|p| p.id),
            project.map(|p| p.directory.clone()).unwrap_or_default(),
        ));
        Ok(TerminalProcess {
            handle: Uuid::new_v4().to_string(),
            input: Box::new(ProjectTerminalInput),
            output: Box::new(ProjectTerminalOutput),
        })
    }
    async fn detach_conversation(
        &self,
        _: &Environment,
        conversation: Uuid,
        _: &str,
    ) -> Result<(), TerminalError> {
        self.detached.lock().unwrap().push(conversation);
        Ok(())
    }
    async fn resize(&self, _: &str, _: TerminalSize) -> Result<(), TerminalError> {
        Ok(())
    }
    async fn detach(&self, _: &Environment, _: &str) -> Result<(), TerminalError> {
        unreachable!()
    }
}

#[tokio::test]
async fn project_chat_routes_context_to_provider_and_terminal_on_resume() {
    let f = ConversationFixture::new().await;
    let environment = f.store.environment.lock().unwrap().clone();
    let project_id = Uuid::new_v4();
    let project = Project {
        id: project_id,
        user_id: environment.user_id,
        repository_id: 123,
        full_name: "owner/repo".into(),
        cloned_full_name: "owner/repo".into(),
        directory: format!("project-{project_id}"),
        state: ProjectState::Ready,
        operation_id: Uuid::new_v4(),
        environment_id: environment.id,
        environment_generation: environment.generation,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    *f.data.2.lock().unwrap() = Some(project.clone());
    let body = json!({"projectId":project_id}).to_string();
    assert_eq!(
        f.call("POST", "/api/conversations", 1, &body)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let create = f.call("POST", "/api/conversations", 0, &body).await;
    assert_eq!(create.status(), StatusCode::OK);
    let created = body_json(create).await;
    assert_eq!(created["projectId"], project_id.to_string());
    let conversation = Uuid::parse_str(created["id"].as_str().unwrap()).unwrap();
    let path = format!("/api/conversations/{conversation}/runs");
    for index in 0..2 {
        let input = json!({"operationId":Uuid::new_v4(),"model":"fixture-model","input":"hello"})
            .to_string();
        assert_eq!(
            f.call("POST", &path, 0, &input).await.status(),
            StatusCode::OK
        );
        until(|| f.provider.project_opens.lock().unwrap().len() == index + 1).await;
        until(|| {
            f.data
                .0
                .lock()
                .unwrap()
                .run
                .as_ref()
                .is_some_and(|r| r.status == RunStatus::Completed)
        })
        .await;
        if index == 0 {
            f.data.0.lock().unwrap().run = None;
        }
    }
    let opens = f.provider.project_opens.lock().unwrap().clone();
    let expected_directory = format!("/home/factory/Documents/{}", project.directory);
    assert_eq!(
        opens,
        vec![
            (project_id, expected_directory.clone(), None),
            (
                project_id,
                expected_directory,
                Some("fixture-project-thread".into())
            )
        ]
    );
    // A failed project preflight settles the fresh run as failed, without a provider session.
    f.data.0.lock().unwrap().run = None;
    f.provider.project_missing.store(true, Ordering::SeqCst);
    let input =
        json!({"operationId":Uuid::new_v4(),"model":"fixture-model","input":"missing"}).to_string();
    assert_eq!(
        f.call("POST", &path, 0, &input).await.status(),
        StatusCode::OK
    );
    until(|| {
        f.data
            .0
            .lock()
            .unwrap()
            .run
            .as_ref()
            .is_some_and(|r| r.status == RunStatus::Failed)
    })
    .await;
    assert_eq!(f.provider.project_opens.lock().unwrap().len(), 2);

    let terminal = Arc::new(ProjectTerminal::default());
    let service = Arc::new(TerminalService::new_with_conversations(
        f.environments.clone(),
        terminal.clone(),
        f.conversation_service.clone(),
    ));
    let app = factory_api::terminals::router(f.auth.clone(), Some(service.clone()));
    let actor = EnvironmentActor {
        user_id: environment.user_id,
        session_hash: f.hashes[0],
    };
    for _ in 0..2 {
        let request = Request::builder()
            .method("POST")
            .uri("/api/terminal/attachments")
            .header("cookie", format!("factory_session={}", f.tokens[0]))
            .header("origin", ORIGIN)
            .header("x-csrf-token", CSRF)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"takeOver":false,"conversationId":conversation}).to_string(),
            ))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let ticket = body_json(response).await["ticket"]
            .as_str()
            .unwrap()
            .to_owned();
        let hash = CredentialHash(Sha256::digest(ticket.as_bytes()).into());
        let attached = service.attach(actor, hash).await.unwrap();
        service.finish(attached).await.unwrap();
    }
    assert_eq!(
        *terminal.attached.lock().unwrap(),
        vec![
            (conversation, Some(project_id), project.directory.clone()),
            (conversation, Some(project_id), project.directory)
        ]
    );
    assert_eq!(
        *terminal.detached.lock().unwrap(),
        vec![conversation, conversation]
    );
}
