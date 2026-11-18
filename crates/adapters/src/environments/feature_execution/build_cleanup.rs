use super::{
    CHECKOUT, DockerEnvironmentRuntime, ExecutionError, WorkerIdentity, collect_check, launch_lock,
    preflight,
};
use factory_core::environments::Environment;
use std::sync::{Arc, atomic::Ordering};

/// Only created for a writable agent/check. No sources, credentials or tooling
/// cache are mounted beside that exact checkout in the cleanup container.
pub(super) struct BuildCleanup {
    env: Environment,
    identity: WorkerIdentity,
}

impl BuildCleanup {
    pub(super) fn new(env: &Environment, identity: &WorkerIdentity) -> Option<Self> {
        Some(Self {
            env: env.clone(),
            identity: identity.cleanup()?,
        })
    }

    pub(super) async fn run(self, runtime: &Arc<DockerEnvironmentRuntime>) {
        if let Err(error) = self.collect(runtime).await {
            // Storage maintenance must not turn a successful check into a failed
            // agent run. Its failure stays visible and retains remaining files.
            tracing::warn!(event = "feature.build_cleanup.failed",
                environment_id = %self.env.id, worker = %self.identity.name,
                execution_error = %error);
        }
    }

    async fn collect(&self, runtime: &Arc<DockerEnvironmentRuntime>) -> Result<(), ExecutionError> {
        let (process, exit) = tokio::time::timeout(runtime.timeout, async {
            let _gate = launch_lock(&self.env).await.map_err(preflight)?;
            runtime.feature_preflight(&self.env).await?;
            let proof = runtime.checkout_proof(&self.env, &self.identity).await?;
            let command = vec![
                "/usr/bin/python3".into(),
                "-I".into(),
                "-c".into(),
                include_str!("build_cleanup.py").into(),
            ];
            let body = self.identity.body(&self.env, CHECKOUT, command)?;
            runtime
                .create_feature_worker(&self.env, &self.identity, body, None, &proof)
                .await
        })
        .await
        .map_err(|_| ExecutionError::CleanupUncertain)??;
        exit.launch_ack.store(true, Ordering::Release);
        // Cleanup workers never get another BuildCleanup. Box the indirect
        // finish -> cleanup -> finish future to keep its type finite.
        let result = Box::pin(collect_check(process, exit)).await?;
        if result.exit_code != 0 {
            return Err(ExecutionError::Unavailable);
        }
        tracing::info!(event = "feature.build_cleanup.completed",
            environment_id = %self.env.id, worker = %self.identity.name);
        Ok(())
    }
}
