use crate::{
    environments::process::{ProcessInput, ProcessOutput},
    json_stream::JsonStream,
};
use factory_core::conversations::execution::ExecutionError;
use serde_json::{Value, json};
use std::{collections::VecDeque, time::Duration};

const MAX_QUEUED: usize = 128;
const IO_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) struct Transport {
    stream: JsonStream,
    queued: VecDeque<Value>,
    next_id: u64,
}
impl Transport {
    pub fn new(input: ProcessInput, output: ProcessOutput) -> Self {
        Self {
            stream: JsonStream::new(input, output),
            queued: VecDeque::new(),
            next_id: 1,
        }
    }
    pub async fn send(&mut self, value: Value) -> Result<(), ExecutionError> {
        self.stream.send(value).await
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
                let frame = self.stream.read().await?;
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
            None => self.stream.read().await,
        }
    }
    pub async fn close(self) -> Result<(), ExecutionError> {
        self.stream.close().await
    }
}
