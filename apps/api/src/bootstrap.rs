use crate::{
    app::{self, IdentityRuntime},
    auth::AuthState,
    config::RuntimeConfig,
};
use factory_adapters::environments::{
    DockerEnvironmentRuntime, InstallationGuard, PreparationMetadata,
};
use factory_adapters::identity::GitHubProvider;
use factory_application::environments::{
    EnvironmentAvailability, EnvironmentReconciler, EnvironmentService,
};
use factory_application::identity::IdentityService;
use factory_core::environments::{EnvironmentError, EnvironmentStart, EnvironmentSupervisor};
use factory_database::environments::PgEnvironmentSupervisor;
use factory_database::{identity::IdentityDatabase, local_target, migrations::Readiness};
use std::{collections::HashMap, sync::Arc, time::Duration};
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
    availability: EnvironmentAvailability,
    stop: watch::Sender<bool>,
    workers: Vec<JoinHandle<()>>,
    _installation: Arc<InstallationGuard>,
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

impl EnvironmentHost {
    pub async fn start(
        config: &RuntimeConfig,
        database: Arc<IdentityDatabase>,
    ) -> Result<Self, EnvironmentError> {
        let directory = config.workspace().join(".factory/environments");
        let metadata = PreparationMetadata::load(&directory)?;
        let installation = Arc::new(InstallationGuard::acquire(&directory)?);
        let request = EnvironmentStart {
            installation_id: metadata.installation_id(),
            image_id: metadata.image_id().to_owned(),
            policy_version: metadata.policy_version().to_owned(),
        };
        let guard = Arc::new(Mutex::new(
            database
                .environment_supervisor(request.installation_id)
                .await?,
        ));
        let availability = EnvironmentAvailability::default();
        let mutation_availability = availability.clone();
        let runtime = Arc::new(DockerEnvironmentRuntime::connect(
            metadata,
            Arc::new(move || !mutation_availability.is_failed()),
        )?);
        let store = Arc::new(database.environment_store());
        let service = Arc::new(EnvironmentService::new(
            store.clone(),
            request.clone(),
            availability.clone(),
        )?);
        let job_guard = guard.clone();
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
        let workers = vec![
            tokio::spawn(reconcile_environments(
                reconciler,
                create_reconciler,
                availability.clone(),
                receiver.clone(),
                installation.clone(),
            )),
            tokio::spawn(monitor_supervisor(
                SharedSupervisor(guard),
                availability.clone(),
                receiver,
                installation.clone(),
            )),
        ];
        Ok(Self {
            service,
            availability,
            stop,
            workers,
            _installation: installation,
        })
    }

    pub fn service(&self) -> Arc<EnvironmentService> {
        self.service.clone()
    }

    /// Drain bounded in-flight operations. Stopping the API never deletes user data.
    pub async fn shutdown(&mut self) {
        self.availability.fail_closed();
        self.stop.send_replace(true);
        for worker in self.workers.drain(..) {
            if worker.await.is_err() {
                tracing::error!(event = "api.environments.worker_failed");
            }
        }
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
                    if let Some(job) = jobs.remove(&id)
                        && !matches!(job.await, Ok(Ok(()))) {
                        availability.fail_closed();
                        tracing::error!(event = "api.environments.worker_failed");
                    }
                }
                if availability.is_failed() { break; }
                let batch = match reconciler.next_batch().await {
                    Ok(batch) => batch,
                    Err(error) => {
                        availability.fail_closed();
                        tracing::warn!(event = "api.environments.supervisor_failed", code = error.code(), guidance = "Restart the API after restoring the owned database.");
                        break;
                    }
                };
                for environment in batch {
                    if jobs.len() == MAX_RECONCILIATION_JOBS { break; }
                    if jobs.contains_key(&environment.id) { continue; }
                    let mut job = match create_reconciler() {
                        Ok(job) => job,
                        Err(_) => {
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
        if job.await.is_err() {
            tracing::error!(event = "api.environments.worker_failed");
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
                    tracing::warn!(event = "api.environments.lock_lost", guidance = "No further runtime operations; restart the API after restoring the owned database.");
                    break;
                }
            }
        }
    }
}

async fn identity(config: &RuntimeConfig) -> Result<IdentityRuntime, &'static str> {
    let settings = config.identity().ok_or("IDENTITY_CONFIGURATION_REQUIRED")?;
    let provider = GitHubProvider::new(
        settings.client_id.clone(),
        settings.client_secret.clone(),
        format!("{}/api/auth/github/callback", config.api_origin()),
    )
    .map_err(|_| "GITHUB_CONFIGURATION_INVALID")?;
    let target = local_target::from_workspace(config.workspace())
        .await
        .map_err(|_| "OWNED_DATABASE_REQUIRED")?;
    let database = Arc::new(
        IdentityDatabase::connect(&target)
            .await
            .map_err(|_| "IDENTITY_DATABASE_UNAVAILABLE")?,
    );
    let code = match database.ready().await {
        Ok(Readiness::Ready) => None,
        Ok(Readiness::Missing | Readiness::Pending) => Some("IDENTITY_MIGRATION_REQUIRED"),
        Ok(Readiness::Invalid) => Some("IDENTITY_SCHEMA_INVALID"),
        Err(_) => Some("IDENTITY_DATABASE_UNAVAILABLE"),
    };
    if let Some(code) = code {
        database.close().await;
        return Err(code);
    }
    let service = Arc::new(IdentityService::new(
        Arc::new(database.store()),
        Arc::new(provider),
    ));
    Ok(IdentityRuntime {
        auth: AuthState::new(service, config),
        database,
    })
}

pub async fn serve(config: RuntimeConfig) -> Result<(), &'static str> {
    let listener = tokio::net::TcpListener::bind(config.bind())
        .await
        .map_err(|_| "API_BIND_FAILED")?;
    let identity = match identity(&config).await {
        Ok(identity) => Some(identity),
        Err(code) => {
            tracing::warn!(
                event = "api.identity.unavailable",
                code,
                guidance = "Check explicit OAuth configuration and owned database; run identity migrations explicitly, then restart."
            );
            None
        }
    };
    let database = identity.as_ref().map(|identity| identity.database.clone());
    let mut environments = if let Some(database) = &database {
        match EnvironmentHost::start(&config, database.clone()).await {
            Ok(host) => Some(host),
            Err(error) => {
                tracing::warn!(
                    event = "api.environments.unavailable",
                    code = error.code(),
                    guidance = "Prepare the local environment image explicitly and ensure only one API uses this installation, then restart."
                );
                None
            }
        }
    } else {
        None
    };
    let router = app::router_with_environments(
        &config,
        identity,
        environments.as_ref().map(EnvironmentHost::service),
    )?;
    let environment_shutdown = environments
        .as_ref()
        .map(|host| (host.availability.clone(), host.stop.clone()));
    tracing::info!(event = "api.listening", origin = config.api_origin());
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown().await;
            if let Some((availability, stop)) = environment_shutdown {
                availability.fail_closed();
                stop.send_replace(true);
            }
        })
        .await
        .map_err(|_| "API_SERVE_FAILED");
    if let Some(host) = &mut environments {
        host.shutdown().await;
    }
    if let Some(database) = database {
        database.close().await;
    }
    result
}

// Adapted from WebApp Template (MIT); no database or mail lifecycle is imported.
async fn shutdown() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
            }
            Err(_) => tracing::error!(event = "api.shutdown_signal.failed"),
        }
    }
    #[cfg(not(unix))]
    if tokio::signal::ctrl_c().await.is_err() {
        tracing::error!(event = "api.shutdown_signal.failed");
    }
}
