use super::{
    ConversationService,
    model_probes::{ModelProbeResult, ModelProbes},
};
use crate::agent_connections::AgentConnectionService;
use factory_core::{
    agent_connections::ConnectionStatus,
    conversations::{
        ConversationError, PendingRequest, RequestKind, RequestState, ResponseState, Run,
        RunStatus,
        execution::{
            ExecutionControl, ExecutionError, ExecutionEvent, ExecutionModel, ExecutionOutput,
            ExecutionQuestion, ExecutionRuntime, QuestionAnswer, validate_answers,
        },
    },
    environments::{Environment, EnvironmentActor},
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex as StdMutex, Weak},
};
use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

/// Host facilities: only the composition layer starts tasks and provides timers.
pub type Task = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
pub type Wait<'a> = Pin<
    Box<dyn Future<Output = Option<Result<Option<ExecutionEvent>, ExecutionError>>> + Send + 'a>,
>;
pub trait ExecutionTicker: Send {
    /// None is a periodic authorization check, Some is provider output.
    fn next<'a>(&'a mut self, output: &'a mut dyn ExecutionOutput) -> Wait<'a>;
}
pub trait ExecutionHost: Send + Sync {
    fn spawn(&self, task: Task);
    fn ticker(&self) -> Box<dyn ExecutionTicker>;
}
/// Keeps durable wire-compatible encoding outside application policy.
pub trait ConversationCodec: Send + Sync {
    fn questions(&self, questions: &[ExecutionQuestion]) -> Result<String, ConversationError>;
    fn answers(&self, answers: &[QuestionAnswer]) -> Result<String, ConversationError>;
}
#[derive(Clone, Debug)]
pub struct ConversationEvent {
    pub run_id: Uuid,
    pub text: Option<String>,
}

struct Active {
    user: Uuid,
    conversation: Uuid,
    control: Mutex<Box<dyn ExecutionControl>>,
    questions: Mutex<HashMap<Uuid, Vec<ExecutionQuestion>>>,
    claims: Mutex<HashMap<Uuid, Uuid>>,
    events: broadcast::Sender<ConversationEvent>,
}

#[derive(Default)]
struct AccountGates(StdMutex<HashMap<Uuid, Weak<Mutex<()>>>>);
impl AccountGates {
    /// Entries live only while an operation holds or waits on the user gate.
    fn for_user(&self, user: Uuid) -> Result<Arc<Mutex<()>>, ConversationError> {
        let mut gates = self.0.lock().map_err(|_| ConversationError::Unavailable)?;
        gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = gates.get(&user).and_then(Weak::upgrade) {
            return Ok(gate);
        }
        if gates.len() >= 256 {
            return Err(ConversationError::Unavailable);
        }
        let gate = Arc::new(Mutex::new(()));
        gates.insert(user, Arc::downgrade(&gate));
        Ok(gate)
    }
}

pub struct ConversationRuntime {
    pub service: Arc<ConversationService>,
    runtime: Arc<dyn ExecutionRuntime>,
    host: Arc<dyn ExecutionHost>,
    codec: Arc<dyn ConversationCodec>,
    accounts: Arc<AgentConnectionService>,
    active: Mutex<HashMap<Uuid, Arc<Active>>>,
    account_gates: AccountGates,
    model_probes: ModelProbes,
    inflight: Mutex<HashMap<Uuid, Uuid>>,
}
impl ConversationRuntime {
    pub fn new(
        service: Arc<ConversationService>,
        runtime: Arc<dyn ExecutionRuntime>,
        accounts: Arc<AgentConnectionService>,
        host: Arc<dyn ExecutionHost>,
        codec: Arc<dyn ConversationCodec>,
    ) -> Arc<Self> {
        Arc::new(Self {
            service,
            runtime,
            host,
            codec,
            accounts,
            active: Mutex::new(HashMap::new()),
            account_gates: AccountGates::default(),
            model_probes: ModelProbes::default(),
            inflight: Mutex::new(HashMap::new()),
        })
    }
    pub async fn models(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Vec<ExecutionModel>, ConversationError> {
        // Every caller authorizes its own session before joining another request.
        let env = self.service.current_environment(actor).await?;
        let probe = self.model_probes.for_environment(&env)?;
        let result = probe
            .result
            .get_or_init(|| async {
                ModelProbeResult {
                    initiator: actor.session_hash,
                    models: self.discover_models(actor, &env).await,
                }
            })
            .await;
        // Waiting must not make an old environment or revoked session usable.
        self.service.same_environment(actor, &env).await?;
        // Another session can initiate the shared probe and expire while it runs.
        // This caller has just authorized itself, so that failure is retryable here.
        result.for_session(actor.session_hash)
    }

    async fn discover_models(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
    ) -> Result<Vec<ExecutionModel>, ConversationError> {
        // Concurrent readers share this operation. Account changes still exclude
        // discovery rather than creating an unbounded queue of provider processes.
        let _gate = self
            .account_gates
            .for_user(actor.user_id)?
            .try_lock_owned()
            .map_err(|_| ConversationError::Conflict)?;
        if self
            .accounts
            .read(actor)
            .await
            .map_err(|_| ConversationError::Unavailable)?
            .status
            != ConnectionStatus::Connected
        {
            return Err(ConversationError::Conflict);
        }
        // Persist the lease before any provider side effect. A crash or lost
        // session leaves it reserved until confirmed cleanup or stopped recovery.
        let lease = Uuid::new_v4();
        self.service.reserve_model_probe(actor, env, lease).await?;
        let result = self.runtime.models(env).await;
        if matches!(result, Err(ExecutionError::LaunchUncertain)) {
            // The daemon may still receive an old start request; stop alone is not proof.
            return Err(ConversationError::Unavailable);
        }
        self.service.settle_model_probe_launch(lease, env).await?;
        if !matches!(result, Err(ExecutionError::CleanupUncertain)) {
            self.service.release_model_probe(actor, lease).await?;
        }
        result.map_err(|_| ConversationError::Unavailable)
    }

    pub async fn begin(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        id: Uuid,
        operation: Uuid,
        model: String,
        input: String,
    ) -> Result<Run, ConversationError> {
        let gate = self.account_gates.for_user(actor.user_id)?;
        let _gate = gate.lock().await;
        let history = self.service.history(actor, id).await?;
        if self
            .accounts
            .read(actor)
            .await
            .map_err(|_| ConversationError::Unavailable)?
            .status
            != ConnectionStatus::Connected
        {
            return Err(ConversationError::Conflict);
        }
        let (env, run, fresh) = self
            .service
            .begin_turn(actor, id, operation, model.clone(), input.clone())
            .await?;
        if !fresh {
            return Ok(run);
        }
        self.inflight.lock().await.insert(run.id, actor.user_id);
        let owner = self.clone();
        let current = run.clone();
        self.host.spawn(Box::pin(async move {
            owner
                .launch(
                    actor,
                    env,
                    current,
                    history.conversation.provider_thread,
                    model,
                    input,
                )
                .await;
        }));
        Ok(run)
    }
    async fn launch(
        self: Arc<Self>,
        actor: EnvironmentActor,
        env: Environment,
        run: Run,
        thread: Option<String>,
        model: String,
        input: String,
    ) {
        let session = match self
            .runtime
            .open(&env, thread.as_deref(), &model, &input)
            .await
        {
            Ok(s) => s,
            Err(error) => {
                let safe_preflight = matches!(
                    error,
                    ExecutionError::PreflightUnavailable | ExecutionError::Invalid
                );
                if safe_preflight {
                    // No process was submitted; recording settlement is safe.
                    if self.service.settle_run_launch(run.id, &env).await.is_ok() {
                        let _ = self
                            .service
                            .finish_turn(actor, &env, run.id, RunStatus::Failed)
                            .await;
                    }
                } else {
                    // Without a conclusive launch response, never claim settlement.
                    let _ = self
                        .service
                        .finish_turn(actor, &env, run.id, RunStatus::Uncertain)
                        .await;
                }
                self.inflight.lock().await.remove(&run.id);
                return;
            }
        };
        if self.service.settle_run_launch(run.id, &env).await.is_err() {
            // Leave the durable reservation fenced; never begin a turn.
            drop(session);
            self.inflight.lock().await.remove(&run.id);
            return;
        }
        let (events, _) = broadcast::channel(128);
        let active = Arc::new(Active {
            user: actor.user_id,
            conversation: run.conversation_id,
            control: Mutex::new(session.control),
            questions: Mutex::new(HashMap::new()),
            claims: Mutex::new(HashMap::new()),
            events,
        });
        self.active.lock().await.insert(run.id, active.clone());
        let mut output = session.output;
        let mut check = self.host.ticker();
        loop {
            let next = match check.next(output.as_mut()).await {
                Some(value) => value,
                None => {
                    if self.service.current_environment(actor).await.is_err() {
                        let _ = active.control.lock().await.interrupt().await;
                        break;
                    }
                    continue;
                }
            };
            let event = match next {
                Ok(Some(e)) => e,
                _ => ExecutionEvent::Uncertain,
            };
            let terminal = matches!(
                event,
                ExecutionEvent::Completed
                    | ExecutionEvent::Failed
                    | ExecutionEvent::Interrupted
                    | ExecutionEvent::Uncertain
            );
            let mut text = None;
            let result = match event {
                ExecutionEvent::ThreadReady { provider_thread } => match self
                    .service
                    .set_thread(actor, &env, run.id, &provider_thread)
                    .await
                {
                    Ok(()) => active
                        .control
                        .lock()
                        .await
                        .begin_turn()
                        .await
                        .map_err(|_| ConversationError::Unavailable),
                    Err(e) => Err(e),
                },
                ExecutionEvent::Delta { text: t, .. } => {
                    text = Some(t);
                    Ok(())
                }
                ExecutionEvent::Message {
                    provider_item,
                    text: t,
                } => self
                    .service
                    .append_assistant(actor, &env, run.id, &provider_item, &t)
                    .await
                    .map(|_| ()),
                ExecutionEvent::Questions {
                    request_id,
                    questions,
                } => match self.codec.questions(&questions) {
                    Ok(details) => {
                        let request = PendingRequest {
                            id: request_id,
                            run_id: run.id,
                            kind: RequestKind::Question,
                            state: RequestState::Pending,
                            details,
                        };
                        match self
                            .service
                            .add_request(actor, &env, request, &request_id.to_string())
                            .await
                        {
                            Ok(()) => {
                                active.questions.lock().await.insert(request_id, questions);
                                Ok(())
                            }
                            Err(e) => Err(e),
                        }
                    }
                    Err(_) => Err(ConversationError::Unavailable),
                },
                ExecutionEvent::ResponseDelivered { request_id } => {
                    let operation = active.claims.lock().await.get(&request_id).copied();
                    match operation {
                        Some(op) => {
                            self.service
                                .record_response(
                                    actor,
                                    &env,
                                    request_id,
                                    op,
                                    ResponseState::Delivered,
                                )
                                .await
                        }
                        None => Err(ConversationError::Conflict),
                    }
                }
                ExecutionEvent::Completed => {
                    self.service
                        .finish_turn(actor, &env, run.id, RunStatus::Completed)
                        .await
                }
                ExecutionEvent::Failed => {
                    self.service
                        .finish_turn(actor, &env, run.id, RunStatus::Failed)
                        .await
                }
                ExecutionEvent::Interrupted => {
                    self.service
                        .finish_turn(actor, &env, run.id, RunStatus::Interrupted)
                        .await
                }
                ExecutionEvent::Uncertain => {
                    self.service
                        .finish_turn(actor, &env, run.id, RunStatus::Uncertain)
                        .await
                }
            };
            if result.is_err() {
                let _ = active.control.lock().await.interrupt().await;
                let _ = self
                    .service
                    .finish_turn(actor, &env, run.id, RunStatus::Uncertain)
                    .await;
                break;
            }
            let _ = active.events.send(ConversationEvent {
                run_id: run.id,
                text,
            });
            if terminal {
                break;
            }
        }
        self.active.lock().await.remove(&run.id);
        self.inflight.lock().await.remove(&run.id);
    }
    pub async fn subscribe(
        &self,
        actor: EnvironmentActor,
        conversation: Uuid,
        run: Uuid,
    ) -> Result<broadcast::Receiver<ConversationEvent>, ConversationError> {
        self.service.history(actor, conversation).await?;
        let active = self
            .active
            .lock()
            .await
            .get(&run)
            .cloned()
            .ok_or(ConversationError::Conflict)?;
        if active.user != actor.user_id || active.conversation != conversation {
            return Err(ConversationError::NotFound);
        }
        Ok(active.events.subscribe())
    }
    pub async fn interrupt(
        &self,
        actor: EnvironmentActor,
        conversation: Uuid,
        run: Uuid,
    ) -> Result<(), ConversationError> {
        self.service.history(actor, conversation).await?;
        let active = self
            .active
            .lock()
            .await
            .get(&run)
            .cloned()
            .ok_or(ConversationError::Conflict)?;
        if active.user != actor.user_id || active.conversation != conversation {
            return Err(ConversationError::NotFound);
        }
        active
            .control
            .lock()
            .await
            .interrupt()
            .await
            .map_err(|_| ConversationError::Unavailable)
    }
    /// Lock login/start across the durable open-run check and first provider response.
    pub async fn acquire_account_change(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Box<dyn Send>, factory_core::agent_connections::AgentConnectionError> {
        let gate = self
            .account_gates
            .for_user(actor.user_id)
            .map_err(|_| factory_core::agent_connections::AgentConnectionError::Unavailable)?
            .lock_owned()
            .await;
        match self.service.has_open_runs(actor).await {
            Ok(false) => Ok(Box::new(gate)),
            Ok(true) => Err(factory_core::agent_connections::AgentConnectionError::Conflict),
            Err(_) => Err(factory_core::agent_connections::AgentConnectionError::Unavailable),
        }
    }
    pub async fn logout(
        &self,
        actor: EnvironmentActor,
    ) -> Result<
        factory_core::agent_connections::ConnectionSnapshot,
        factory_core::agent_connections::AgentConnectionError,
    > {
        let gate = self
            .account_gates
            .for_user(actor.user_id)
            .map_err(|_| factory_core::agent_connections::AgentConnectionError::Unavailable)?;
        let _gate = gate.lock().await;
        match self.service.has_open_runs(actor).await {
            Ok(false) => self.accounts.logout(actor).await,
            Ok(true) => Err(factory_core::agent_connections::AgentConnectionError::Conflict),
            Err(_) => Err(factory_core::agent_connections::AgentConnectionError::Unavailable),
        }
    }
    pub async fn has_active(&self, user: Uuid) -> bool {
        self.inflight.lock().await.values().any(|id| *id == user)
    }
    pub async fn answer(
        &self,
        actor: EnvironmentActor,
        conversation: Uuid,
        run: Uuid,
        request: Uuid,
        operation: Uuid,
        answers: Vec<QuestionAnswer>,
    ) -> Result<(), ConversationError> {
        self.service.history(actor, conversation).await?;
        let active = self
            .active
            .lock()
            .await
            .get(&run)
            .cloned()
            .ok_or(ConversationError::Conflict)?;
        if active.user != actor.user_id || active.conversation != conversation {
            return Err(ConversationError::NotFound);
        }
        let questions = active
            .questions
            .lock()
            .await
            .get(&request)
            .cloned()
            .ok_or(ConversationError::Conflict)?;
        validate_answers(&questions, &answers).map_err(|_| ConversationError::Invalid)?;
        let encoded = self.codec.answers(&answers)?;
        let env = self.service.current_environment(actor).await?;
        let claim = self
            .service
            .claim_response(actor, &env, request, operation, &encoded)
            .await?;
        if !claim.fresh {
            return if claim.state == ResponseState::Delivered {
                Ok(())
            } else {
                Err(ConversationError::Conflict)
            };
        }
        active.claims.lock().await.insert(request, operation);
        if active
            .control
            .lock()
            .await
            .answer(request, answers)
            .await
            .is_err()
        {
            let _ = self
                .service
                .record_response(actor, &env, request, operation, ResponseState::Uncertain)
                .await;
            return Err(ConversationError::Unavailable);
        }
        Ok(())
    }
}

#[cfg(test)]
mod account_gate_tests {
    use super::*;

    #[tokio::test]
    async fn account_changes_serialize_per_user_without_blocking_other_users() {
        let gates = AccountGates::default();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();
        let held = gates.for_user(alice).unwrap().lock_owned().await;
        let same = gates.for_user(alice).unwrap();
        let other = gates.for_user(bob).unwrap();
        assert!(same.try_lock().is_err());
        assert!(other.try_lock().is_ok());
        drop(held);
        assert!(same.try_lock().is_ok());
    }

    #[test]
    fn unused_account_gates_are_reclaimed_and_concurrent_users_are_bounded() {
        let gates = AccountGates::default();
        let held = (0..256)
            .map(|_| gates.for_user(Uuid::new_v4()).unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            gates.for_user(Uuid::new_v4()),
            Err(ConversationError::Unavailable)
        ));
        drop(held);
        assert!(gates.for_user(Uuid::new_v4()).is_ok());
        assert_eq!(gates.0.lock().unwrap().len(), 1);
    }
}
