use super::recovery::RecoveryHost;
use crate::config::RuntimeConfig;
use crate::conversations::runtime::ConversationRuntime;
use factory_adapters::claude_account::ClaudeAccountAdapter;
use factory_adapters::claude_execution::ClaudeExecutionAdapter;
use factory_adapters::codex_account::CodexAccountAdapter;
use factory_adapters::codex_execution::CodexExecutionAdapter;
use factory_adapters::egress::EgressMetadata;
use factory_adapters::environments::{
    DockerEnvironmentRuntime, InstallationGuard, PreparationMetadata,
};
use factory_adapters::terminals::DockerTerminalRuntime;
use factory_application::agent_connections::AgentConnectionService;
use factory_application::conversations::ConversationService;
use factory_application::conversations::coordinator::ExecutionProvider;
use factory_application::environments::{
    EnvironmentAvailability, EnvironmentReconciler, EnvironmentService,
};
use factory_application::terminals::TerminalService;
use factory_core::environments::{EnvironmentError, EnvironmentStart, EnvironmentSupervisor};
use factory_database::environments::PgEnvironmentSupervisor;
use factory_database::identity::IdentityDatabase;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Mutex, watch},
    task::JoinHandle,
};
use uuid::Uuid;

const MAX_RECONCILIATION_JOBS: usize = 32;

/// Composes the same supervised lifecycle for the server and isolated integration hosts.
/// Keep this owner alive until HTTP shutdown; its file lock survives a lost DB guard.
pub struct EnvironmentHost {
    service: Arc<EnvironmentService>,
    terminal: Arc<TerminalService>,
    accounts: Arc<AgentConnectionService>,
    claude_accounts: Arc<AgentConnectionService>,
    conversations: Arc<ConversationRuntime>,
    availability: EnvironmentAvailability,
    stop: watch::Sender<bool>,
    workers: Vec<JoinHandle<()>>,
    _installation: Arc<InstallationGuard>,
    fatal: Arc<AtomicBool>,
}

struct SharedSupervisor(Arc<Mutex<PgEnvironmentSupervisor>>);

struct UnavailableOnExit(EnvironmentAvailability);

impl Drop for UnavailableOnExit {
    fn drop(&mut self) {
        self.0.fail_closed();
    }
}

#[async_trait::async_trait]
impl EnvironmentSupervisor for SharedSupervisor {
    async fn check(&mut self) -> Result<(), EnvironmentError> {
        self.0.lock().await.check().await
    }
}

/// Prepared once; recovery never rereads configuration or releases the process lock.
#[derive(Clone)]
pub(super) struct EnvironmentHostFactory {
    database: Arc<IdentityDatabase>,
    metadata: PreparationMetadata,
    egress: EgressMetadata,
    installation: Arc<InstallationGuard>,
    state: Arc<StdMutex<FactoryState>>,
}

#[derive(Default)]
struct FactoryState {
    closed: bool,
    current: Option<EnvironmentAvailability>,
}

impl EnvironmentHostFactory {
    pub(super) fn prepare(
        config: &RuntimeConfig,
        database: Arc<IdentityDatabase>,
    ) -> Result<Self, EnvironmentError> {
        let directory = config.workspace().join(".factory/environments");
        let metadata = PreparationMetadata::load(&directory)?;
        let egress = EgressMetadata::load(&directory)?;
        let installation = Arc::new(InstallationGuard::acquire(&directory)?);
        Ok(Self {
            database,
            metadata,
            egress,
            installation,
            state: Arc::new(StdMutex::new(FactoryState::default())),
        })
    }

    pub(super) fn close(&self) {
        // Publication and shutdown may race across threads. Registering each
        // epoch here makes shutdown synchronously revoke even an unpublished one.
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.closed = true;
        if let Some(current) = state.current.take() {
            current.fail_closed();
        }
    }

    pub(super) async fn start(&self) -> Result<EnvironmentHost, EnvironmentError> {
        if self
            .state
            .lock()
            .map_err(|_| EnvironmentError::Unavailable)?
            .closed
        {
            return Err(EnvironmentError::Unavailable);
        }
        let request = EnvironmentStart {
            installation_id: self.metadata.installation_id(),
            image_id: self.metadata.image_id().to_owned(),
            policy_version: self.metadata.policy_version().to_owned(),
        };
        let guard = Arc::new(Mutex::new(
            self.database
                .environment_supervisor(request.installation_id)
                .await?,
        ));
        let availability = EnvironmentAvailability::default();
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| EnvironmentError::Unavailable)?;
            if state.closed {
                return Err(EnvironmentError::Unavailable);
            }
            if let Some(previous) = state.current.replace(availability.clone()) {
                previous.fail_closed();
            }
        }
        let mutation_availability = availability.clone();
        let runtime = Arc::new(DockerEnvironmentRuntime::connect(
            self.metadata.clone(),
            self.egress.clone(),
            Arc::new(move || !mutation_availability.is_failed()),
        )?);
        let store = Arc::new(self.database.environment_store());
        let service = Arc::new(EnvironmentService::new(
            store.clone(),
            request.clone(),
            availability.clone(),
        )?);
        let terminal = Arc::new(TerminalService::new(
            service.clone(),
            Arc::new(DockerTerminalRuntime::new(runtime.clone())),
        ));
        let job_guard = guard.clone();
        let accounts = Arc::new(AgentConnectionService::new(
            service.clone(),
            Arc::new(CodexAccountAdapter::new(runtime.clone())),
        ));
        let claude_accounts = Arc::new(AgentConnectionService::new(
            service.clone(),
            Arc::new(ClaudeAccountAdapter::new(runtime.clone())),
        ));
        let conversations = crate::conversations::runtime::with_providers(
            Arc::new(ConversationService::new(
                service.clone(),
                Arc::new(self.database.conversation_store()),
            )),
            vec![
                ExecutionProvider {
                    id: "codex",
                    display_name: "Codex",
                    accounts: accounts.clone(),
                    runtime: Arc::new(CodexExecutionAdapter::new(runtime.clone())),
                },
                ExecutionProvider {
                    id: "claude",
                    display_name: "Claude",
                    accounts: claude_accounts.clone(),
                    runtime: Arc::new(ClaudeExecutionAdapter::new(runtime.clone())),
                },
            ],
        )
        .map_err(|_| EnvironmentError::InvalidConfiguration)?;
        let job_availability = availability.clone();
        let create_reconciler = move || {
            EnvironmentReconciler::new(
                store.clone(),
                runtime.clone(),
                Box::new(SharedSupervisor(job_guard.clone())),
                request.clone(),
                job_availability.clone(),
            )
        };
        let reconciler = create_reconciler()?;
        let (stop, receiver) = watch::channel(false);
        let fatal = Arc::new(AtomicBool::new(false));
        let workers = vec![
            tokio::spawn(reconcile_environments(
                reconciler,
                create_reconciler,
                availability.clone(),
                receiver.clone(),
                self.installation.clone(),
                fatal.clone(),
            )),
            tokio::spawn(monitor_supervisor(
                SharedSupervisor(guard),
                availability.clone(),
                receiver,
                self.installation.clone(),
            )),
        ];
        Ok(EnvironmentHost {
            service,
            terminal,
            accounts,
            claude_accounts,
            conversations,
            availability,
            stop,
            workers,
            _installation: self.installation.clone(),
            fatal,
        })
    }
}

impl EnvironmentHost {
    pub fn service(&self) -> Arc<EnvironmentService> {
        self.service.clone()
    }

    pub fn terminal(&self) -> Arc<TerminalService> {
        self.terminal.clone()
    }

    pub fn accounts(&self) -> Arc<AgentConnectionService> {
        self.accounts.clone()
    }

    pub fn claude_accounts(&self) -> Arc<AgentConnectionService> {
        self.claude_accounts.clone()
    }

    pub(crate) fn conversations(&self) -> Arc<ConversationRuntime> {
        self.conversations.clone()
    }

    /// Drain bounded in-flight operations. Stopping the API never deletes user data.
    pub async fn shutdown(&mut self) {
        self.close();
        for worker in self.workers.drain(..) {
            if worker.await.is_err() {
                self.fatal.store(true, Ordering::SeqCst);
                tracing::error!(event = "api.environments.worker_failed");
            }
        }
    }
}

#[async_trait::async_trait]
impl RecoveryHost for EnvironmentHost {
    fn is_failed(&self) -> bool {
        self.availability.is_failed()
    }
    fn is_ready(&self) -> bool {
        self.availability.is_available()
    }

    fn close(&self) {
        self.availability.fail_closed();
        self.stop.send_replace(true);
    }

    async fn drain(&mut self) -> bool {
        self.shutdown().await;
        !self.fatal.load(Ordering::SeqCst)
    }
}

impl Drop for EnvironmentHost {
    fn drop(&mut self) {
        self.availability.fail_closed();
        self.stop.send_replace(true);
        // Workers retain the file lock while an already-sent Docker operation settles.
    }
}

async fn reconcile_environments(
    mut reconciler: EnvironmentReconciler,
    create_reconciler: impl Fn() -> Result<EnvironmentReconciler, EnvironmentError>,
    availability: EnvironmentAvailability,
    mut stop: watch::Receiver<bool>,
    installation: Arc<InstallationGuard>,
    fatal: Arc<AtomicBool>,
) {
    let _on_exit = UnavailableOnExit(availability.clone());
    let mut jobs: HashMap<Uuid, JoinHandle<Result<(), EnvironmentError>>> = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            _ = interval.tick() => {
                if *stop.borrow() || availability.is_failed() { break; }
                let completed: Vec<_> = jobs.iter()
                    .filter_map(|(id, job)| job.is_finished().then_some(*id)).collect();
                for id in completed {
                    if let Some(job) = jobs.remove(&id) {
                        observe_job(job.await, &availability, &fatal);
                    }
                }
                if availability.is_failed() { break; }
                let batch = match reconciler.next_batch().await {
                    Ok(batch) => batch,
                    Err(error) => {
                        availability.fail_closed();
                        if !super::recovery::retryable(error) { fatal.store(true, Ordering::SeqCst); }
                        tracing::warn!(event = "api.environments.supervisor_failed", code = error.code(), guidance = "Runtime admission is closed while supervision recovers.");
                        break;
                    }
                };
                for environment in batch {
                    if jobs.len() == MAX_RECONCILIATION_JOBS { break; }
                    if jobs.contains_key(&environment.id) { continue; }
                    let mut job = match create_reconciler() {
                        Ok(job) => job,
                        Err(_) => {
                            fatal.store(true, Ordering::SeqCst);
                            availability.fail_closed();
                            break;
                        }
                    };
                    // A slow Docker operation must not hold up another user's stop.
                    // Detached handles retain the process lock even if this loop panics.
                    let installation = installation.clone();
                    jobs.insert(environment.id, tokio::spawn(async move {
                        let _installation = installation;
                        job.reconcile_environment(environment).await
                    }));
                }
            }
        }
    }
    availability.fail_closed();
    for job in jobs.into_values() {
        observe_job(job.await, &availability, &fatal);
    }
}

fn observe_job(
    result: Result<Result<(), EnvironmentError>, tokio::task::JoinError>,
    availability: &EnvironmentAvailability,
    fatal: &AtomicBool,
) {
    match result {
        Ok(Ok(())) => (),
        Ok(Err(error)) => {
            availability.fail_closed();
            if !super::recovery::retryable(error) {
                fatal.store(true, Ordering::SeqCst);
            }
            tracing::warn!(
                event = "api.environments.worker_failed",
                code = error.code()
            );
        }
        Err(_) => {
            fatal.store(true, Ordering::SeqCst);
            availability.fail_closed();
            tracing::error!(event = "api.environments.worker_panicked");
        }
    }
}

async fn monitor_supervisor(
    mut guard: SharedSupervisor,
    availability: EnvironmentAvailability,
    mut stop: watch::Receiver<bool>,
    _installation: Arc<InstallationGuard>,
) {
    let _on_exit = UnavailableOnExit(availability.clone());
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            _ = interval.tick() => {
                if *stop.borrow() || availability.is_failed() { break; }
                if guard.check().await.is_err() {
                    availability.fail_closed();
                    tracing::warn!(event = "api.environments.lock_lost", guidance = "Runtime admission is closed; supervision will reacquire ownership before resuming.");
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
