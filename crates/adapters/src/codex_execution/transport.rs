use crate::environments::process::{ProcessInput, ProcessOutput};
use bollard::container::LogOutput;
use factory_core::conversations::execution::ExecutionError;
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{collections::VecDeque, time::Duration};
use tokio::io::AsyncWriteExt;

const MAX_FRAME: usize = 1024 * 1024;
const MAX_QUEUED: usize = 128;
const IO_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) struct Transport {
    input: ProcessInput,
    output: ProcessOutput,
    bytes: Vec<u8>,
    queued: VecDeque<Value>,
    next_id: u64,
}
impl Transport {
    pub fn new(input: ProcessInput, output: ProcessOutput) -> Self {
        Self {
            input,
            output,
            bytes: Vec::new(),
            queued: VecDeque::new(),
            next_id: 1,
        }
    }
    pub async fn send(&mut self, value: Value) -> Result<(), ExecutionError> {
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
    pub async fn initialize(&mut self) -> Result<(), ExecutionError> {
        let value = self
            .request(
                "initialize",
                json!({"clientInfo":{"name":"factory","version":env!("CARGO_PKG_VERSION")}}),
            )
            .await?;
        if !value.get("userAgent").is_some_and(Value::is_string) {
            return Err(ExecutionError::Protocol);
        }
        self.send(json!({"method":"initialized"})).await
    }
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, ExecutionError> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(ExecutionError::Protocol)?;
        self.send(json!({"id":id,"method":method,"params":params}))
            .await?;
        tokio::time::timeout(IO_TIMEOUT, async {
            loop {
                let frame = self.read().await?;
                if frame.get("method").is_none() {
                    if frame.get("id").and_then(Value::as_u64) != Some(id) {
                        return Err(ExecutionError::Protocol);
                    }
                    if frame.get("error").is_some() {
                        return Err(ExecutionError::Unavailable);
                    }
                    return frame.get("result").cloned().ok_or(ExecutionError::Protocol);
                }
                if self.queued.len() == MAX_QUEUED {
                    return Err(ExecutionError::Protocol);
                }
                self.queued.push_back(frame);
            }
        })
        .await
        .map_err(|_| ExecutionError::Unavailable)?
    }
    pub async fn next(&mut self) -> Result<Value, ExecutionError> {
        match self.queued.pop_front() {
            Some(value) => Ok(value),
            None => self.read().await,
        }
    }
    async fn read(&mut self) -> Result<Value, ExecutionError> {
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
                // Stderr may contain private configuration and must never reach the browser or logs.
                LogOutput::StdErr { .. } => (),
                _ => return Err(ExecutionError::Protocol),
            }
        }
    }
    pub async fn close(mut self) -> Result<(), ExecutionError> {
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
    #[test]
    fn rejects_non_objects_deep_or_large_frames() {
        assert!(parse(b"[]").is_err());
        assert!(
            parse(format!("{{\"x\":{}}}", "[".repeat(34) + "0" + &"]".repeat(34)).as_bytes())
                .is_err()
        );
        assert!(parse(&vec![b' '; MAX_FRAME + 1]).is_err());
    }
}
