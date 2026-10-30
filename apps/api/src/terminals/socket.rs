use super::contracts::*;
use crate::auth::{AuthState, Authenticated, hash_token};
use axum::extract::ws::{Message, WebSocket};
use chrono::Utc;
use factory_application::{
    identity::IdentityRevocation,
    terminals::{TerminalAttachment, TerminalService},
};
use factory_core::{environments::EnvironmentActor, terminals::*};
use futures_util::SinkExt;
use std::{sync::Arc, time::Duration};
use tokio::{sync::broadcast, time::Instant};

const IO_TIMEOUT: Duration = Duration::from_secs(3);
const REVALIDATION_INTERVAL: Duration = Duration::from_secs(5);
const ACK_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) async fn run(
    mut socket: WebSocket,
    auth: AuthState,
    authenticated: Authenticated,
    service: Arc<TerminalService>,
    mut revocations: broadcast::Receiver<IdentityRevocation>,
) {
    let actor = EnvironmentActor {
        user_id: authenticated.session.user.id,
        session_hash: authenticated.token_hash,
    };
    let expiry = Instant::now()
        + (authenticated.session.expires_at - Utc::now())
            .to_std()
            .unwrap_or_default();
    let opening = async {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.recv())
            .await
            .map_err(|_| TerminalError::InvalidTicket)?
            .ok_or(TerminalError::InvalidTicket)?
            .map_err(|_| TerminalError::InvalidTicket)?;
        let Message::Text(text) = message else {
            return Err(TerminalError::InvalidTicket);
        };
        if text.len() > MAX_CONTROL_BYTES {
            return Err(TerminalError::InvalidTicket);
        }
        let TerminalClientMessage::Authenticate { ticket } =
            serde_json::from_str(&text).map_err(|_| TerminalError::InvalidTicket)?
        else {
            return Err(TerminalError::InvalidTicket);
        };
        let hash = hash_token(&ticket).map_err(|_| TerminalError::InvalidTicket)?;
        let session = auth
            .service()
            .session(actor.session_hash)
            .await
            .map_err(|_| TerminalError::Unauthorized)?;
        if session.user.id != actor.user_id {
            return Err(TerminalError::Unauthorized);
        }
        Ok(hash)
    };
    let ticket = tokio::select! {
        biased;
        _ = tokio::time::sleep_until(expiry) => Err(TerminalError::Unauthorized),
        _ = revoked(&mut revocations, actor) => Err(TerminalError::Unauthorized),
        result = opening => result,
    };
    let ticket = match ticket {
        Ok(ticket) => ticket,
        Err(error) => {
            let _ = control(
                &mut socket,
                TerminalServerMessage::Error { code: error.into() },
            )
            .await;
            return;
        }
    };
    // Once Docker creation may have been dispatched, revocation must not drop
    // this future and lose the returned handle. Close browser access immediately,
    // settle the bounded operation, then detach any resulting process.
    let mut attaching = Box::pin(service.attach(actor, ticket));
    let (opened, interrupted) = tokio::select! {
        biased;
        _ = tokio::time::sleep_until(expiry) => {
            let _ = control(&mut socket, detached(TerminalErrorCode::Unauthorized)).await;
            let _ = tokio::time::timeout(IO_TIMEOUT, socket.close()).await;
            (attaching.await, true)
        },
        _ = revoked(&mut revocations, actor) => {
            let _ = control(&mut socket, detached(TerminalErrorCode::Unauthorized)).await;
            let _ = tokio::time::timeout(IO_TIMEOUT, socket.close()).await;
            (attaching.await, true)
        },
        result = &mut attaching => (result, false),
    };
    let mut attachment = match opened {
        Ok(attachment) => attachment,
        Err(error) => {
            if !interrupted {
                let _ = control(
                    &mut socket,
                    TerminalServerMessage::Error { code: error.into() },
                )
                .await;
            }
            return;
        }
    };
    if interrupted {
        finish(&mut socket, &service, attachment).await;
        return;
    }
    let outcome = tokio::select! {
        biased;
        _ = tokio::time::sleep_until(expiry) => TerminalServerMessage::Detached { code: TerminalErrorCode::Unauthorized },
        _ = revoked(&mut revocations, actor) => TerminalServerMessage::Detached { code: TerminalErrorCode::Unauthorized },
        result = pump(&mut socket, &service, &mut attachment) => result,
    };
    // Notify before potentially slow Docker verification; keep the final error
    // honest if that verification fails. No terminal process is killed here.
    let _ = control(&mut socket, outcome).await;
    finish(&mut socket, &service, attachment).await;
    let _ = tokio::time::timeout(IO_TIMEOUT, socket.close()).await;
}

async fn finish(socket: &mut WebSocket, service: &TerminalService, attachment: TerminalAttachment) {
    if service.finish(attachment).await.is_err() {
        tracing::warn!(event = "api.terminal.cleanup_uncertain");
        let _ = control(
            socket,
            TerminalServerMessage::Error {
                code: TerminalErrorCode::CleanupUncertain,
            },
        )
        .await;
    }
}

async fn revoked(receiver: &mut broadcast::Receiver<IdentityRevocation>, actor: EnvironmentActor) {
    loop {
        match receiver.recv().await {
            Ok(event) if !event.matches(actor.user_id, actor.session_hash) => continue,
            // A lagged receiver cannot prove it did not miss this session's event.
            _ => return,
        }
    }
}

async fn pump(
    socket: &mut WebSocket,
    service: &TerminalService,
    attachment: &mut TerminalAttachment,
) -> TerminalServerMessage {
    if let Err(error) = service.check(attachment).await {
        return detached(error.into());
    }
    if control(socket, TerminalServerMessage::Ready).await.is_err() {
        return detached(TerminalErrorCode::Unavailable);
    }
    let mut validation = tokio::time::interval(REVALIDATION_INTERVAL);
    validation.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut window = OutputWindow::default();
    loop {
        let ack_deadline = window
            .deadline
            .unwrap_or_else(|| Instant::now() + ACK_TIMEOUT);
        tokio::select! {
            biased;
            _ = attachment.replaced.changed() => return detached(TerminalErrorCode::Conflict),
            _ = validation.tick() => {
                if let Err(error) = service.check(attachment).await { return detached(error.into()); }
            },
            _ = tokio::time::sleep_until(ack_deadline), if window.pending > 0 => return detached(TerminalErrorCode::SlowClient),
            incoming = socket.recv() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(Message::Close(_))) | None => return detached(TerminalErrorCode::Unavailable),
                    _ => return detached(TerminalErrorCode::InvalidInput),
                };
                let message = match serde_json::from_str::<TerminalClientMessage>(&text) {
                    Ok(message) => message,
                    Err(_) => return detached(TerminalErrorCode::InvalidInput),
                };
                let result = match message {
                    TerminalClientMessage::Input { data } => service.write(attachment, data.as_bytes()).await,
                    TerminalClientMessage::Resize { cols, rows } if text.len() <= MAX_CONTROL_BYTES => {
                        match TerminalSize::new(cols, rows) {
                            Ok(size) => service.resize(attachment, size).await,
                            Err(error) => Err(error),
                        }
                    },
                    TerminalClientMessage::Ack { bytes } if text.len() <= MAX_CONTROL_BYTES => window.ack(bytes as usize),
                    _ => Err(TerminalError::InvalidInput),
                };
                if let Err(error) = result { return detached(error.into()); }
            },
            output = attachment.output.read(), if window.can_read() => {
                match output {
                    Ok(Some(bytes)) if !bytes.is_empty() && bytes.len() <= OUTPUT_CHUNK_BYTES => {
                        window.sent(bytes.len());
                        if !matches!(tokio::time::timeout(IO_TIMEOUT, socket.send(Message::Binary(bytes.into()))).await, Ok(Ok(()))) {
                            return detached(TerminalErrorCode::SlowClient);
                        }
                    },
                    Ok(None) => return TerminalServerMessage::Exit,
                    _ => return detached(TerminalErrorCode::Unavailable),
                }
            },
        }
    }
}

fn detached(code: TerminalErrorCode) -> TerminalServerMessage {
    TerminalServerMessage::Detached { code }
}

async fn control(socket: &mut WebSocket, message: TerminalServerMessage) -> Result<(), ()> {
    let text = serde_json::to_string(&message).map_err(|_| ())?;
    tokio::time::timeout(IO_TIMEOUT, socket.send(Message::Text(text.into())))
        .await
        .map_err(|_| ())?
        .map_err(|_| ())
}

#[derive(Default)]
struct OutputWindow {
    pending: usize,
    deadline: Option<Instant>,
}

impl OutputWindow {
    fn can_read(&self) -> bool {
        self.pending <= OUTPUT_WINDOW_BYTES - OUTPUT_CHUNK_BYTES
    }
    fn sent(&mut self, bytes: usize) {
        if self.pending == 0 {
            self.deadline = Some(Instant::now() + ACK_TIMEOUT);
        }
        self.pending += bytes;
    }
    fn ack(&mut self, bytes: usize) -> Result<(), TerminalError> {
        if bytes == 0 || bytes > self.pending {
            return Err(TerminalError::InvalidInput);
        }
        self.pending -= bytes;
        self.deadline = (self.pending > 0).then(|| Instant::now() + ACK_TIMEOUT);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_window_requires_real_acknowledgements_and_never_exceeds_its_limit() {
        let mut window = OutputWindow::default();
        for _ in 0..OUTPUT_WINDOW_BYTES / OUTPUT_CHUNK_BYTES {
            assert!(window.can_read());
            window.sent(OUTPUT_CHUNK_BYTES);
        }
        assert!(!window.can_read());
        assert!(window.ack(OUTPUT_WINDOW_BYTES + 1).is_err());
        assert!(window.ack(0).is_err());
        window.ack(OUTPUT_CHUNK_BYTES).unwrap();
        assert!(window.can_read());
        window
            .ack(OUTPUT_WINDOW_BYTES - OUTPUT_CHUNK_BYTES)
            .unwrap();
        assert!(window.deadline.is_none());
    }
}
