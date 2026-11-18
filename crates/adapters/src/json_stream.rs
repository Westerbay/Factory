//! Bounded JSON-lines transport for provider processes. Protocols own correlation.
use crate::environments::process::{ProcessInput, ProcessOutput};
use bollard::container::LogOutput;
use factory_core::conversations::execution::ExecutionError;
use futures_util::StreamExt;
use serde_json::Value;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const MAX_FRAME: usize = 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(20);
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) struct JsonStream {
    input: ProcessInput,
    output: ProcessOutput,
    bytes: Vec<u8>,
    read_failure: Option<&'static str>,
    repository_pending: bool,
}

impl JsonStream {
    pub(crate) fn new(input: ProcessInput, output: ProcessOutput) -> Self {
        Self {
            input,
            output,
            bytes: Vec::new(),
            read_failure: None,
            repository_pending: false,
        }
    }

    pub(crate) fn prepared(input: ProcessInput, output: ProcessOutput) -> Self {
        Self {
            repository_pending: true,
            ..Self::new(input, output)
        }
    }

    /// This is separate from the provider's protocol deadline. Fresh checkouts
    /// may still be installing locked dependencies before a native CLI exists.
    pub(crate) async fn repository_ready(&mut self) -> Result<(), ExecutionError> {
        if !self.repository_pending {
            return Ok(());
        }
        let frame = match tokio::time::timeout(ACTIVATION_TIMEOUT, self.read()).await {
            Ok(result) => result?,
            Err(_) => {
                self.record_timeout("repository_activation_timeout");
                return Err(ExecutionError::RepositoryPreparationFailed(
                    "activation-timeout",
                ));
            }
        };
        match frame["type"].as_str() {
            Some("factory_repository_ready") => {
                self.repository_pending = false;
                Ok(())
            }
            Some("factory_repository_failed") => {
                let step = match frame["step"].as_str() {
                    Some("environment-not-prepared") => "environment-not-prepared",
                    Some("node-offline-install") => "node-offline-install",
                    Some("disk-space") => "disk-space",
                    Some("filesystem-permission") => "filesystem-permission",
                    Some("executable-format") => "executable-format",
                    _ => "configuration",
                };
                Err(ExecutionError::RepositoryPreparationFailed(step))
            }
            _ => {
                self.read_failure = Some("invalid_repository_activation_frame");
                Err(ExecutionError::Protocol)
            }
        }
    }

    pub(crate) fn record_timeout(&mut self, reason: &'static str) {
        self.read_failure = Some(reason);
    }

    pub(crate) async fn send(&mut self, value: Value) -> Result<(), ExecutionError> {
        let mut bytes = serde_json::to_vec(&value).map_err(|_| ExecutionError::Protocol)?;
        if bytes.len() > MAX_FRAME {
            return Err(ExecutionError::Invalid);
        }
        bytes.push(b'\n');
        tokio::time::timeout(IO_TIMEOUT, async {
            self.input.write_all(&bytes).await?;
            self.input.flush().await
        })
        .await
        .map_err(|_| ExecutionError::Unavailable)?
        .map_err(|_| ExecutionError::Unavailable)
    }

    pub(crate) async fn read(&mut self) -> Result<Value, ExecutionError> {
        let mut discarded_stderr = 0usize;
        loop {
            if let Some(i) = self.bytes.iter().position(|b| *b == b'\n') {
                let bytes: Vec<_> = self.bytes.drain(..=i).collect();
                if bytes.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                return parse(&bytes).inspect_err(|_| {
                    self.read_failure = Some("invalid_json_frame");
                });
            }
            let chunk = match self.output.next().await {
                Some(Ok(chunk)) => chunk,
                Some(Err(_)) => {
                    self.read_failure = Some("output_stream_error");
                    return Err(ExecutionError::Unavailable);
                }
                None => {
                    self.read_failure = Some("unexpected_eof");
                    return Err(ExecutionError::Unavailable);
                }
            };
            match chunk {
                LogOutput::StdOut { message } | LogOutput::Console { message } => {
                    if self.bytes.len() + message.len() > MAX_FRAME {
                        self.read_failure = Some("output_frame_limit");
                        return Err(ExecutionError::Protocol);
                    }
                    self.bytes.extend_from_slice(&message);
                }
                // Provider diagnostics may contain private configuration or credentials.
                LogOutput::StdErr { message } => {
                    discarded_stderr = discarded_stderr.saturating_add(message.len());
                    if discarded_stderr > MAX_FRAME {
                        self.read_failure = Some("stderr_limit");
                        return Err(ExecutionError::Protocol);
                    }
                }
                _ => {
                    self.read_failure = Some("unexpected_output_stream");
                    return Err(ExecutionError::Protocol);
                }
            }
        }
    }

    pub(crate) fn read_failure(&self) -> Option<&'static str> {
        self.read_failure
    }

    pub(crate) async fn close(mut self) -> Result<(), ExecutionError> {
        tokio::time::timeout(Duration::from_secs(3), self.input.shutdown())
            .await
            .map_err(|_| ExecutionError::CleanupUncertain)?
            .map_err(|_| ExecutionError::CleanupUncertain)
    }
}

fn parse(bytes: &[u8]) -> Result<Value, ExecutionError> {
    fn depth(v: &Value, n: usize) -> bool {
        n <= 32
            && match v {
                Value::Object(m) => m.values().all(|v| depth(v, n + 1)),
                Value::Array(a) => a.iter().all(|v| depth(v, n + 1)),
                _ => true,
            }
    }
    if bytes.len() > MAX_FRAME {
        return Err(ExecutionError::Protocol);
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ExecutionError::Protocol)?;
    if !value.is_object() || !depth(&value, 0) {
        return Err(ExecutionError::Protocol);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[tokio::test]
    async fn repository_readiness_preserves_batched_provider_frames_and_sanitizes_failures() {
        let output = stream::iter(vec![Ok(LogOutput::StdOut {
            message: b"{\"type\":\"factory_repository_ready\"}\n{\"id\":1}\n"
                .to_vec()
                .into(),
        })]);
        let mut stream = JsonStream::prepared(Box::pin(tokio::io::sink()), Box::pin(output));
        stream.repository_ready().await.unwrap();
        stream.repository_ready().await.unwrap();
        assert_eq!(stream.read().await.unwrap()["id"], 1);
        for (step, expected) in [
            ("node-offline-install", "node-offline-install"),
            ("private credential", "configuration"),
        ] {
            let output = stream::iter(vec![Ok(LogOutput::StdOut {
                message: format!(
                    "{{\"type\":\"factory_repository_failed\",\"step\":\"{step}\"}}\n"
                )
                .into(),
            })]);
            let mut stream = JsonStream::prepared(Box::pin(tokio::io::sink()), Box::pin(output));
            assert_eq!(
                stream.repository_ready().await,
                Err(ExecutionError::RepositoryPreparationFailed(expected))
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn repository_activation_is_bounded_without_claiming_a_protocol_timeout() {
        let mut stream =
            JsonStream::prepared(Box::pin(tokio::io::sink()), Box::pin(stream::pending()));
        assert_eq!(
            stream.repository_ready().await,
            Err(ExecutionError::RepositoryPreparationFailed(
                "activation-timeout"
            ))
        );
        assert_eq!(stream.read_failure(), Some("repository_activation_timeout"));
    }

    #[test]
    fn rejects_non_objects_deep_or_large_frames() {
        assert!(parse(b"[]").is_err());
        assert!(
            parse(format!("{{\"x\":{}}}", "[".repeat(34) + "0" + &"]".repeat(34)).as_bytes())
                .is_err()
        );
        assert!(parse(&vec![b' '; MAX_FRAME + 1]).is_err());
    }

    #[tokio::test]
    async fn fragmented_and_batched_frames_ignore_private_stderr() {
        let output = stream::iter(vec![
            Ok(LogOutput::StdErr {
                message: "private diagnostic".into(),
            }),
            Ok(LogOutput::StdOut {
                message: "{\"id\":".into(),
            }),
            Ok(LogOutput::StdOut {
                message: "1}\n{\"id\":2}\n".into(),
            }),
        ]);
        let mut stream = JsonStream::new(Box::pin(tokio::io::sink()), Box::pin(output));
        assert_eq!(stream.read().await.unwrap()["id"], 1);
        assert_eq!(stream.read().await.unwrap()["id"], 2);
    }

    #[tokio::test]
    async fn excessive_private_stderr_fails_without_exposing_its_contents() {
        let output = stream::iter(vec![Ok(LogOutput::StdErr {
            message: vec![b'x'; MAX_FRAME + 1].into(),
        })]);
        let mut stream = JsonStream::new(Box::pin(tokio::io::sink()), Box::pin(output));
        assert_eq!(stream.read().await, Err(ExecutionError::Protocol));
    }
}
