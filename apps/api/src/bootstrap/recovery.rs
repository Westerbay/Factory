//! Replace a failed runtime as one unit; never reopen an old host's permissions.
use axum::Router;
use factory_core::environments::EnvironmentError;
use std::{future::Future, time::Duration};
use tokio::{sync::watch, task::JoinHandle};
use tower::ServiceExt;

const RETRY_INTERVAL: Duration = Duration::from_secs(1);
const MAX_RETRY_INTERVAL: Duration = Duration::from_secs(30);

#[async_trait::async_trait]
pub(super) trait RecoveryHost: Send {
    fn is_failed(&self) -> bool;
    fn is_ready(&self) -> bool;
    /// Permanently revoke this host before draining it or publishing a replacement.
    fn close(&self);
    /// False denotes a panic or permanent failure, which must not restart automatically.
    async fn drain(&mut self) -> bool;
}

pub(super) struct HostRouter<H> {
    pub router: Router,
    pub host: H,
}

pub(super) fn retryable(error: EnvironmentError) -> bool {
    matches!(
        error,
        EnvironmentError::Unavailable | EnvironmentError::Timeout | EnvironmentError::Conflict
    )
}

pub(super) fn recovering_router<H, Create, Creating>(
    fallback: Router,
    initial: Option<HostRouter<H>>,
    create: Create,
    stop: watch::Receiver<bool>,
) -> (Router, JoinHandle<()>)
where
    H: RecoveryHost + 'static,
    Create: FnMut() -> Creating + Send + 'static,
    Creating: Future<Output = Result<HostRouter<H>, EnvironmentError>> + Send + 'static,
{
    let (published, current) = watch::channel(
        initial
            .as_ref()
            .map_or(fallback, |epoch| epoch.router.clone()),
    );
    // Snapshot per request, including requests sharing a keep-alive connection.
    // No watch read guard survives an await or blocks publication.
    let router = Router::new().fallback_service(tower::service_fn(move |request| {
        let router = current.borrow().clone();
        async move { router.oneshot(request).await }
    }));
    let worker = tokio::spawn(supervise(initial, published, create, stop, RETRY_INTERVAL));
    (router, worker)
}

fn stopping(stop: &watch::Receiver<bool>) -> bool {
    *stop.borrow() || stop.has_changed().is_err()
}

async fn supervise<H, Create, Creating>(
    mut current: Option<HostRouter<H>>,
    published: watch::Sender<Router>,
    mut create: Create,
    mut stop: watch::Receiver<bool>,
    interval: Duration,
) where
    H: RecoveryHost,
    Create: FnMut() -> Creating,
    Creating: Future<Output = Result<HostRouter<H>, EnvironmentError>>,
{
    let mut retry_delay = interval;
    while !stopping(&stop) {
        if current
            .as_ref()
            .is_some_and(|epoch| !epoch.host.is_failed())
        {
            if current.as_ref().is_some_and(|epoch| epoch.host.is_ready()) {
                retry_delay = interval;
            }
            tokio::select! {
                biased;
                _ = stop.changed() => break,
                _ = tokio::time::sleep(interval) => continue,
            }
        }
        if let Some(mut old) = current.take() {
            old.host.close();
            if !old.host.drain().await {
                tracing::error!(
                    event = "api.environments.recovery_stopped",
                    guidance = "A runtime worker failed permanently; inspect the error before restarting the API."
                );
                break;
            }
        }
        if stopping(&stop) {
            break;
        }
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            _ = tokio::time::sleep(retry_delay) => (),
        }
        // Cancelling acquisition drops its dedicated connection. A host is only
        // returned after its workers exist; a completed candidate is drained below.
        let candidate = tokio::select! {
            biased;
            _ = stop.changed() => break,
            candidate = create() => candidate,
        };
        match candidate {
            Ok(mut next) => {
                if stopping(&stop) {
                    next.host.close();
                    next.host.drain().await;
                    break;
                }
                drop(published.send_replace(next.router.clone()));
                current = Some(next);
                // Connection ownership alone does not prove healthy supervision.
                // Reset only after a guarded sweep has actually reached readiness.
                retry_delay = (retry_delay * 2).min(MAX_RETRY_INTERVAL);
                tracing::info!(
                    event = "api.environments.ownership_reacquired",
                    guidance = "The new supervisor will verify environment state before allowing interactive work."
                );
            }
            Err(error) if retryable(error) => {
                retry_delay = (retry_delay * 2).min(MAX_RETRY_INTERVAL);
                tracing::warn!(
                    event = "api.environments.recovery_pending",
                    code = error.code(),
                    retry_seconds = retry_delay.as_secs()
                );
            }
            Err(error) => {
                tracing::error!(
                    event = "api.environments.recovery_stopped",
                    code = error.code()
                );
                break;
            }
        }
    }
    if let Some(mut current) = current {
        current.host.close();
        current.host.drain().await;
    }
}

#[cfg(test)]
mod tests;
