//! Operator diagnostics derived from typed errors, never from provider messages.
use factory_core::conversations::execution::{ExecutionError, ExecutionEvent};
use uuid::Uuid;

pub(crate) struct ExecutionDiagnostics {
    provider: &'static str,
    attempt_id: Option<Uuid>,
    pub(crate) phase: &'static str,
    pub(crate) provider_error: Option<&'static str>,
    pub(crate) result_subtype: Option<&'static str>,
    pub(crate) http_status: Option<u16>,
    pub(crate) stream_error: Option<&'static str>,
    pub(crate) rpc_error: Option<(&'static str, i64)>,
}

impl ExecutionDiagnostics {
    pub(crate) fn new(provider: &'static str, attempt_id: Option<Uuid>) -> Self {
        Self {
            provider,
            attempt_id,
            phase: "initialize",
            provider_error: None,
            result_subtype: None,
            http_status: None,
            rpc_error: None,
            stream_error: None,
        }
    }

    pub(crate) fn report(
        &self,
        terminal: &ExecutionEvent,
        protocol: Option<ExecutionError>,
        close: Option<ExecutionError>,
        exit: Result<i64, ExecutionError>,
    ) {
        let outcome = match terminal {
            ExecutionEvent::Failed => "failed",
            ExecutionEvent::Uncertain => "uncertain",
            _ => return,
        };
        tracing::warn!(
            event = "agent.execution.failed",
            provider = self.provider,
            attempt_id = self.attempt_id.as_ref().map(tracing::field::display),
            outcome,
            phase = self.phase,
            provider_error = self.provider_error,
            result_subtype = self.result_subtype,
            http_status = self.http_status,
            stream_error = self.stream_error,
            rpc_method = self.rpc_error.map(|(method, _)| method),
            rpc_code = self.rpc_error.map(|(_, code)| code),
            execution_error = protocol.as_ref().map(tracing::field::display),
            transport_close_error = close.as_ref().map(tracing::field::display),
            exit_code = exit.ok(),
            exit_error = exit.as_ref().err().map(tracing::field::display),
        );
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };

    pub(crate) struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

    impl CapturedLogs {
        pub(crate) fn capture() -> (Self, tracing::subscriber::DefaultGuard) {
            let logs = Arc::new(Mutex::new(Vec::new()));
            let writer = Arc::clone(&logs);
            let subscriber = tracing_subscriber::fmt()
                .json()
                .with_writer(move || Writer(Arc::clone(&writer)))
                .finish();
            (Self(logs), tracing::subscriber::set_default(subscriber))
        }

        pub(crate) fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
