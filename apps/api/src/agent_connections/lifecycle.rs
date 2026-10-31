use crate::auth::{AuthState, Authenticated};
use chrono::{DateTime, Utc};
use factory_application::{
    agent_connections::AgentConnectionService, identity::IdentityRevocation,
};
use factory_core::{
    agent_connections::{AgentConnectionError, ConnectionSnapshot},
    environments::EnvironmentActor,
};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{
    sync::{broadcast, oneshot},
    time::Instant,
};
use uuid::Uuid;

/// A dropped HTTP handler must not abandon a process after Docker has created it.
/// The provider's bounded operation continues through its own cleanup.
pub(super) async fn settle<T: Send + 'static>(
    operation: impl Future<Output = Result<T, AgentConnectionError>> + Send + 'static,
) -> Result<T, AgentConnectionError> {
    tokio::spawn(operation)
        .await
        .map_err(|_| AgentConnectionError::Unavailable)?
}

pub(super) async fn begin(
    auth: AuthState,
    service: Arc<AgentConnectionService>,
    authenticated: Authenticated,
    held: Box<dyn Send>,
) -> Result<ConnectionSnapshot, AgentConnectionError> {
    let actor = EnvironmentActor {
        user_id: authenticated.session.user.id,
        session_hash: authenticated.token_hash,
    };
    let session_expiry = authenticated.session.expires_at;
    let (sender, receiver) = oneshot::channel();
    let revocations = auth.service().subscribe_revocations();
    tokio::spawn(start_and_monitor(
        auth,
        service,
        actor,
        session_expiry,
        revocations,
        sender,
        held,
    ));
    receiver
        .await
        .map_err(|_| AgentConnectionError::Unavailable)?
}

async fn start_and_monitor(
    auth: AuthState,
    service: Arc<AgentConnectionService>,
    actor: EnvironmentActor,
    session_expiry: DateTime<Utc>,
    mut revocations: broadcast::Receiver<IdentityRevocation>,
    sender: oneshot::Sender<Result<ConnectionSnapshot, AgentConnectionError>>,
    held: Box<dyn Send>,
) {
    let mut reply = Some(sender);
    let mut starting = Box::pin(service.begin_login(actor, session_expiry));
    let expiry = deadline(session_expiry);
    let (result, interrupted) = tokio::select! {
        biased;
        _ = reply.as_mut().expect("reply pending").closed() => (starting.await, true),
        _ = tokio::time::sleep_until(expiry) => {
            reject(&mut reply, AgentConnectionError::Unauthorized);
            (starting.await, true)
        },
        _ = revoked(&mut revocations, actor) => {
            reject(&mut reply, AgentConnectionError::Unauthorized);
            (starting.await, true)
        },
        result = &mut starting => (result, false),
    };
    let snapshot = match result {
        Ok(snapshot) => snapshot,
        Err(error) => {
            reject(&mut reply, error);
            return;
        }
    };
    let Some(challenge) = snapshot.challenge.clone() else {
        reject(&mut reply, AgentConnectionError::Protocol);
        return;
    };
    let operation_id = challenge.operation_id;
    if interrupted {
        interrupt(&service, actor, operation_id).await;
        return;
    }
    // Never publish a device code after a queued start lost its authorization.
    let authorized = tokio::select! {
        biased;
        _ = reply.as_mut().expect("reply pending").closed() => Err(AgentConnectionError::Unauthorized),
        _ = tokio::time::sleep_until(expiry) => Err(AgentConnectionError::Unauthorized),
        _ = revoked(&mut revocations, actor) => Err(AgentConnectionError::Unauthorized),
        result = revalidate(service.clone(), actor, operation_id) => result,
    };
    match authorized {
        Err(error) => {
            reject(&mut reply, error);
            interrupt(&service, actor, operation_id).await;
            return;
        }
        Ok(false) => {
            // Sign-in may have completed before its first HTTP response. Read
            // the final account instead of exposing a stale authorization code.
            let result = service.read(actor).await;
            if session_expiry <= Utc::now()
                || auth.service().session(actor.session_hash).await.is_err()
            {
                reject(&mut reply, AgentConnectionError::Unauthorized);
                return;
            }
            if let Some(reply) = reply.take() {
                let _ = reply.send(result);
            }
            return;
        }
        Ok(true) => {}
    }
    if reply
        .take()
        .expect("reply pending")
        .send(Ok(snapshot))
        .is_err()
    {
        interrupt(&service, actor, operation_id).await;
        return;
    }
    drop(held);
    let poll = async {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match revalidate(service.clone(), actor, operation_id).await {
                Ok(true) => {}
                Ok(false) => return false,
                Err(_) => return true,
            }
        }
    };
    let should_interrupt = tokio::select! {
        biased;
        _ = tokio::time::sleep_until(deadline(challenge.expires_at.min(session_expiry))) => true,
        _ = revoked(&mut revocations, actor) => true,
        result = poll => result,
    };
    if should_interrupt {
        interrupt(&service, actor, operation_id).await;
    }
}

async fn revalidate(
    service: Arc<AgentConnectionService>,
    actor: EnvironmentActor,
    operation_id: Uuid,
) -> Result<bool, AgentConnectionError> {
    settle(async move { service.revalidate_pending(actor, operation_id).await }).await
}

fn deadline(at: DateTime<Utc>) -> Instant {
    Instant::now() + (at - Utc::now()).to_std().unwrap_or_default()
}

fn reject(
    reply: &mut Option<oneshot::Sender<Result<ConnectionSnapshot, AgentConnectionError>>>,
    error: AgentConnectionError,
) {
    if let Some(reply) = reply.take() {
        let _ = reply.send(Err(error));
    }
}

async fn revoked(receiver: &mut broadcast::Receiver<IdentityRevocation>, actor: EnvironmentActor) {
    loop {
        match receiver.recv().await {
            Ok(event) if !event.matches(actor.user_id, actor.session_hash) => continue,
            _ => return,
        }
    }
}

async fn interrupt(service: &AgentConnectionService, actor: EnvironmentActor, operation_id: Uuid) {
    if let Err(error) = service.interrupt(actor, operation_id).await {
        tracing::warn!(
            event = "api.agent_connection.interruption_uncertain",
            code = error.code()
        );
    }
}
