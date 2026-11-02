mod environments;
mod recovery;
mod repository_key;

use crate::{
    app::{self, IdentityRuntime},
    auth::AuthState,
    config::RuntimeConfig,
};
use environments::{EnvironmentHost, EnvironmentHostFactory};
use factory_adapters::identity::GitHubProvider;
use factory_application::identity::IdentityService;
use factory_core::environments::EnvironmentError;
use factory_database::{identity::IdentityDatabase, local_target, schema::Readiness};
use recovery::{HostRouter, RecoveryHost};
use std::sync::Arc;
use tokio::sync::watch;

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
        Ok(Readiness::Missing) => Some("IDENTITY_SCHEMA_REQUIRED"),
        Ok(Readiness::Invalid) => Some("IDENTITY_SCHEMA_INVALID"),
        Err(_) => Some("IDENTITY_DATABASE_UNAVAILABLE"),
    };
    if let Some(code) = code {
        database.close().await;
        return Err(code);
    }
    let store = Arc::new(database.store());
    let service = Arc::new(IdentityService::new(store, Arc::new(provider)));
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
                guidance = "Check explicit OAuth configuration and owned database; initialize the database, then restart."
            );
            None
        }
    };
    let database = identity.as_ref().map(|identity| identity.database.clone());
    let api_origin = config.api_origin().to_owned();
    // The prepared factory holds the installation lock for the entire server
    // lifetime, including outages, recovery attempts and an unexpected worker exit.
    let factory = if let Some(database) = &database {
        match EnvironmentHostFactory::prepare(&config, database.clone()).await {
            Ok(factory) => Some(factory),
            Err(error) => {
                tracing::warn!(
                    event = "api.environments.unavailable",
                    code = error.code(),
                    guidance = "Prepare the local environment and ensure only one API uses this installation, then restart."
                );
                None
            }
        }
    } else {
        None
    };
    let (stop, stopped) = watch::channel(false);
    let fallback = app::router(&config, identity.clone())?;
    let (router, recovery) = if let Some(factory) = factory.clone() {
        let create = move || {
            let factory = factory.clone();
            let config = config.clone();
            let identity = identity.clone();
            async move { create_host_router(&factory, &config, identity).await }
        };
        match create().await {
            Err(error) if !recovery::retryable(error) => {
                tracing::error!(
                    event = "api.environments.recovery_stopped",
                    code = error.code()
                );
                (fallback, None)
            }
            initial => {
                if let Err(error) = &initial {
                    tracing::warn!(
                        event = "api.environments.recovery_pending",
                        code = error.code()
                    );
                }
                let (router, worker) =
                    recovery::recovering_router(fallback, initial.ok(), create, stopped);
                (router, Some(worker))
            }
        }
    } else {
        (fallback, None)
    };
    tracing::info!(event = "api.listening", origin = api_origin);
    let shutdown_stop = stop.clone();
    let shutdown_factory = factory.clone();
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            shutdown().await;
            if let Some(factory) = shutdown_factory {
                factory.close();
            }
            shutdown_stop.send_replace(true);
        })
        .await
        .map_err(|_| "API_SERVE_FAILED");
    if let Some(factory) = &factory {
        factory.close();
    }
    stop.send_replace(true);
    if let Some(worker) = recovery
        && worker.await.is_err()
    {
        tracing::error!(event = "api.environments.recovery_worker_failed");
    }
    drop(factory);
    if let Some(database) = database {
        database.close().await;
    }
    result
}

async fn create_host_router(
    factory: &EnvironmentHostFactory,
    config: &RuntimeConfig,
    identity: Option<IdentityRuntime>,
) -> Result<HostRouter<EnvironmentHost>, EnvironmentError> {
    let mut host = factory.start().await?;
    let router = app::router_with_projects(
        config,
        identity,
        Some(host.service()),
        Some(host.terminal()),
        (Some(host.accounts()), Some(host.claude_accounts())),
        Some(host.conversations()),
        Some(host.projects()),
    );
    match router {
        Ok(router) => Ok(HostRouter { router, host }),
        Err(_) => {
            host.close();
            host.drain().await;
            Err(EnvironmentError::InvalidConfiguration)
        }
    }
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

#[cfg(test)]
mod integration;
