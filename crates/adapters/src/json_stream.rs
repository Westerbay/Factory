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

pub(crate) struct JsonStream {
    input: ProcessInput,
    output: ProcessOutput,
    bytes: Vec<u8>,
}

impl JsonStream {
    pub(crate) fn new(input: ProcessInput, output: ProcessOutput) -> Self {
        Self {
            input,
            output,
            bytes: Vec::new(),
        }
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
                return parse(&bytes);
            }
            let chunk = self
                .output
                .next()
                .await
                .ok_or(ExecutionError::Unavailable)?
                .map_err(|_| ExecutionError::Unavailable)?;
            match chunk {
                LogOutput::StdOut { message } | LogOutput::Console { message } => {
                    if self.bytes.len() + message.len() > MAX_FRAME {
                        return Err(ExecutionError::Protocol);
                    }
                    self.bytes.extend_from_slice(&message);
                }
                // Provider diagnostics may contain private configuration or credentials.
                LogOutput::StdErr { message } => {
                    discarded_stderr = discarded_stderr.saturating_add(message.len());
                    if discarded_stderr > MAX_FRAME {
                        return Err(ExecutionError::Protocol);
                    }
                }
                _ => return Err(ExecutionError::Protocol),
            }
        }
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
