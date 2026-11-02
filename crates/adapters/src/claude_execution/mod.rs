//! Claude Code's pinned JSON-lines process protocol in an owned environment.
use crate::{
    environments::{
        DockerEnvironmentRuntime,
        process::{ClaudeLaunch, DockerProcess, FixedProcess, ProcessStartError},
    },
    json_stream::JsonStream,
    projects::{DockerProjectRuntime, ProjectDirectory},
};
use async_trait::async_trait;
use factory_core::{
    conversations::execution::*,
    environments::Environment,
    projects::{Project, ProjectState},
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

const LIMIT: usize = 64;
const MAX_TEXT: usize = 256 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ClaudeExecutionAdapter {
    runtime: Arc<DockerEnvironmentRuntime>,
}

impl ClaudeExecutionAdapter {
    pub fn new(runtime: Arc<DockerEnvironmentRuntime>) -> Self {
        Self { runtime }
    }

    async fn process(
        &self,
        env: &Environment,
        command: FixedProcess,
    ) -> Result<(String, JsonStream), ExecutionError> {
        if !self
            .runtime
            .egress()
            .available()
            .await
            .map_err(|_| ExecutionError::PreflightUnavailable)?
        {
            return Err(ExecutionError::PreflightUnavailable);
        }
        let DockerProcess { id, input, output } = self
            .runtime
            .start_process_checked(env, command)
            .await
            .map_err(|error| match error {
                ProcessStartError::PreSubmission(_) => ExecutionError::PreflightUnavailable,
                ProcessStartError::Submitted(_) => ExecutionError::LaunchUncertain,
            })?;
        Ok((id, JsonStream::new(input, output)))
    }
}

#[async_trait]
impl ExecutionRuntime for ClaudeExecutionAdapter {
    async fn models(&self, env: &Environment) -> Result<Vec<ExecutionModel>, ExecutionError> {
        let (id, mut stream) = self.process(env, FixedProcess::ClaudeProbe).await?;
        let result = initialize(&mut stream)
            .await
            .and_then(|response| parse_models(&response));
        let close = stream.close().await;
        let exit = self
            .runtime
            .wait_process_exit(&id, Duration::from_secs(15))
            .await
            .map_err(|_| ExecutionError::CleanupUncertain)?;
        close?;
        if exit != 0 {
            return Err(ExecutionError::Unavailable);
        }
        result
    }

    async fn open(
        &self,
        env: &Environment,
        provider_thread: Option<&str>,
        model: &str,
        effort: Option<&str>,
        input: &str,
    ) -> Result<ExecutionSession, ExecutionError> {
        self.open_project(env, None, provider_thread, model, effort, input)
            .await
    }

    async fn open_project(
        &self,
        env: &Environment,
        project: Option<&Project>,
        provider_thread: Option<&str>,
        model: &str,
        effort: Option<&str>,
        input: &str,
    ) -> Result<ExecutionSession, ExecutionError> {
        if input.trim().is_empty() || input.len() > MAX_TEXT || input.contains('\0') {
            return Err(ExecutionError::Invalid);
        }
        let session = match provider_thread {
            Some(value) => Uuid::parse_str(value).map_err(|_| ExecutionError::Invalid)?,
            None => Uuid::new_v4(),
        };
        let mut launch = ClaudeLaunch::new(model, effort, session, provider_thread.is_some())
            .ok_or(ExecutionError::Invalid)?;
        if let Some(project) = project {
            if project.state != ProjectState::Ready
                || DockerProjectRuntime::new(self.runtime.clone())
                    .inspect_project(env, project)
                    .await
                    != Ok(ProjectDirectory::Ready)
            {
                return Err(ExecutionError::PreflightUnavailable);
            }
            launch = launch.with_cwd(format!("/home/factory/Documents/{}", project.directory));
        }
        let (id, stream) = self
            .process(env, FixedProcess::ClaudeExecution(launch))
            .await?;
        let (commands, receiver) = mpsc::channel(LIMIT);
        let (sender, events) = mpsc::channel(LIMIT);
        let runtime = self.runtime.clone();
        let input = input.to_owned();
        tokio::spawn(async move {
            let (result, stream) = Worker::new(stream, receiver, sender.clone(), session, input)
                .run()
                .await;
            let closed = stream.close().await;
            let exited = runtime
                .wait_process_exit(&id, Duration::from_secs(15))
                .await;
            let terminal = match (result, closed, exited) {
                (Ok(event), Ok(()), Ok(0)) => event,
                _ => ExecutionEvent::Uncertain,
            };
            let _ = emit(&sender, terminal).await;
        });
        Ok(ExecutionSession {
            control: Box::new(Control { commands }),
            output: Box::new(Output { events }),
        })
    }
}

async fn initialize(stream: &mut JsonStream) -> Result<Value, ExecutionError> {
    stream.send(json!({"type":"control_request","request_id":"factory-init","request":{"subtype":"initialize","hooks":{},"mcpServers":{}}})).await?;
    tokio::time::timeout(IO_TIMEOUT, async {
        let frame = stream.read().await?;
        if frame["type"] != "control_response"
            || frame["response"]["request_id"] != "factory-init"
            || frame["response"]["subtype"] != "success"
        {
            return Err(ExecutionError::Protocol);
        }
        frame["response"]["response"]
            .as_object()
            .map(|_| frame["response"]["response"].clone())
            .ok_or(ExecutionError::Protocol)
    })
    .await
    .map_err(|_| ExecutionError::Unavailable)?
}

fn parse_models(response: &Value) -> Result<Vec<ExecutionModel>, ExecutionError> {
    let models = response["models"]
        .as_array()
        .ok_or(ExecutionError::Protocol)?;
    if models.is_empty() || models.len() > 1000 {
        return Err(ExecutionError::Protocol);
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for model in models {
        let id = text(&model["value"], 256)?;
        let name = text(&model["displayName"], 512)?;
        if ClaudeLaunch::new(id, None, Uuid::from_u128(1), false).is_none() {
            return Err(ExecutionError::Protocol);
        }
        if seen.insert(id.to_owned()) {
            let resolved_model = model
                .get("resolvedModel")
                .filter(|value| !value.is_null())
                .map(|value| text(value, 256).map(str::to_owned))
                .transpose()?;
            let effort_levels = match model.get("supportedEffortLevels") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(levels)) if levels.len() <= 20 => levels
                    .iter()
                    .map(|value| {
                        text(value, 32).and_then(|level| {
                            if valid_effort(level) {
                                Ok(level.to_owned())
                            } else {
                                Err(ExecutionError::Protocol)
                            }
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                _ => return Err(ExecutionError::Protocol),
            };
            result.push(ExecutionModel {
                id: id.to_owned(),
                name: name.to_owned(),
                resolved_model,
                effort_levels,
                default_effort: None,
            });
        }
    }
    Ok(result)
}

fn text(value: &Value, max: usize) -> Result<&str, ExecutionError> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= max && !s.chars().any(char::is_control))
        .ok_or(ExecutionError::Protocol)
}
fn content_text(value: &Value, max: usize) -> Result<&str, ExecutionError> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= max && !s.contains('\0'))
        .ok_or(ExecutionError::Protocol)
}
async fn emit(
    tx: &mpsc::Sender<ExecutionEvent>,
    event: ExecutionEvent,
) -> Result<(), ExecutionError> {
    tokio::time::timeout(Duration::from_secs(5), tx.send(event))
        .await
        .map_err(|_| ExecutionError::Unavailable)?
        .map_err(|_| ExecutionError::Unavailable)
}

type Reply = oneshot::Sender<Result<(), ExecutionError>>;
enum Command {
    Begin(Reply),
    Answer(Uuid, Vec<QuestionAnswer>, Reply),
    Interrupt(Reply),
}
struct Control {
    commands: mpsc::Sender<Command>,
}
impl Control {
    async fn call(&mut self, f: impl FnOnce(Reply) -> Command) -> Result<(), ExecutionError> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(f(tx))
            .await
            .map_err(|_| ExecutionError::Unavailable)?;
        tokio::time::timeout(Duration::from_secs(25), rx)
            .await
            .map_err(|_| ExecutionError::Unavailable)?
            .map_err(|_| ExecutionError::Unavailable)?
    }
}
#[async_trait]
impl ExecutionControl for Control {
    async fn begin_turn(&mut self) -> Result<(), ExecutionError> {
        self.call(Command::Begin).await
    }
    async fn answer(
        &mut self,
        id: Uuid,
        answers: Vec<QuestionAnswer>,
    ) -> Result<(), ExecutionError> {
        self.call(|reply| Command::Answer(id, answers, reply)).await
    }
    async fn interrupt(&mut self) -> Result<(), ExecutionError> {
        self.call(Command::Interrupt).await
    }
}
struct Output {
    events: mpsc::Receiver<ExecutionEvent>,
}
#[async_trait]
impl ExecutionOutput for Output {
    async fn next(&mut self) -> Result<Option<ExecutionEvent>, ExecutionError> {
        Ok(self.events.recv().await)
    }
}

struct Worker {
    stream: JsonStream,
    commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<ExecutionEvent>,
    session: Uuid,
    input: String,
    current_item: Option<String>,
    tasks: HashSet<String>,
    result: Option<ExecutionEvent>,
    stopping: bool,
    stop_deadline: Option<tokio::time::Instant>,
    requests: HashMap<Uuid, PendingQuestion>,
}
struct PendingQuestion {
    native_id: String,
    tool_id: String,
    input: Value,
    questions: Vec<ExecutionQuestion>,
    sent: bool,
}
impl Worker {
    fn new(
        stream: JsonStream,
        commands: mpsc::Receiver<Command>,
        events: mpsc::Sender<ExecutionEvent>,
        session: Uuid,
        input: String,
    ) -> Self {
        Self {
            stream,
            commands,
            events,
            session,
            input,
            current_item: None,
            tasks: HashSet::new(),
            result: None,
            stopping: false,
            stop_deadline: None,
            requests: HashMap::new(),
        }
    }
    async fn run(mut self) -> (Result<ExecutionEvent, ExecutionError>, JsonStream) {
        let result = self.work().await;
        (result, self.stream)
    }
    async fn work(&mut self) -> Result<ExecutionEvent, ExecutionError> {
        initialize(&mut self.stream).await?;
        emit(
            &self.events,
            ExecutionEvent::ThreadReady {
                provider_thread: self.session.to_string(),
            },
        )
        .await?;
        match tokio::time::timeout(IO_TIMEOUT, self.commands.recv())
            .await
            .map_err(|_| ExecutionError::Unavailable)?
        {
            Some(Command::Begin(reply)) => {
                let result = self.stream.send(json!({"type":"user","message":{"role":"user","content":self.input},"parent_tool_use_id":null})).await;
                let _ = reply.send(result);
                result?;
            }
            _ => return Err(ExecutionError::Unavailable),
        }
        loop {
            if self.result.is_some() && self.tasks.is_empty() {
                return Ok(self.result.take().expect("result checked"));
            }
            let deadline = self.stop_deadline;
            tokio::select! {
                _ = async {
                    match deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending::<()>().await,
                    }
                } => return Err(ExecutionError::Unavailable),
                command = self.commands.recv() => match command {
                    Some(Command::Interrupt(reply)) => {
                        if self.stopping {
                            let _ = reply.send(Ok(()));
                            continue;
                        }
                        let result = self.stream.send(json!({"type":"control_request","request_id":"factory-interrupt","request":{"subtype":"interrupt"}})).await;
                        if result.is_ok() {
                            self.stopping = true;
                            self.stop_deadline = Some(tokio::time::Instant::now() + Duration::from_secs(20));
                        }
                        let _ = reply.send(result);
                        result?;
                    },
                    Some(Command::Answer(id, answers, reply)) => {
                        let result = self.answer(id, answers).await;
                        let _ = reply.send(result);
                    },
                    Some(Command::Begin(reply)) => { let _ = reply.send(Err(ExecutionError::Conflict)); },
                    None => return Err(ExecutionError::Unavailable),
                },
                frame = self.stream.read() => self.frame(frame?).await?,
            }
        }
    }
    async fn frame(&mut self, frame: Value) -> Result<(), ExecutionError> {
        let kind = text(&frame["type"], 80)?;
        if let Some(session) = frame.get("session_id").and_then(Value::as_str)
            && session != self.session.to_string()
        {
            return Err(ExecutionError::Protocol);
        }
        match kind {
            "system" => match frame["subtype"].as_str() {
                Some("task_started") => {
                    self.tasks.insert(text(&frame["task_id"], 512)?.to_owned());
                    if self.tasks.len() > LIMIT {
                        return Err(ExecutionError::Protocol);
                    }
                }
                Some("background_tasks_changed") => {
                    let tasks = frame["tasks"].as_array().ok_or(ExecutionError::Protocol)?;
                    if tasks.len() > LIMIT {
                        return Err(ExecutionError::Protocol);
                    }
                    for task in tasks {
                        self.tasks.insert(text(&task["task_id"], 512)?.to_owned());
                    }
                    if self.tasks.len() > LIMIT {
                        return Err(ExecutionError::Protocol);
                    }
                }
                Some("task_notification") => {
                    if !self.tasks.remove(text(&frame["task_id"], 512)?) {
                        return Err(ExecutionError::Protocol);
                    }
                }
                _ => {}
            },
            "stream_event" if frame["parent_tool_use_id"].is_null() => {
                let event = &frame["event"];
                match event["type"].as_str() {
                    Some("message_start") => {
                        self.current_item = Some(text(&event["message"]["id"], 512)?.to_owned());
                    }
                    Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                        let delta = event["delta"]["text"]
                            .as_str()
                            .filter(|value| value.len() <= MAX_TEXT && !value.contains('\0'))
                            .ok_or(ExecutionError::Protocol)?;
                        if delta.is_empty() {
                            return Ok(());
                        }
                        let item = self.current_item.clone().ok_or(ExecutionError::Protocol)?;
                        emit(
                            &self.events,
                            ExecutionEvent::Delta {
                                provider_item: item,
                                text: delta.to_owned(),
                            },
                        )
                        .await?;
                    }
                    _ => {}
                }
            }
            "assistant" if frame["parent_tool_use_id"].is_null() => {
                // Claude can emit several assistant frames with the same message ID.
                let item = text(&frame["uuid"], 512)?.to_owned();
                let content = frame["message"]["content"]
                    .as_array()
                    .ok_or(ExecutionError::Protocol)?;
                let mut message = String::new();
                for block in content {
                    if block["type"] == "text" {
                        let part = block["text"].as_str().ok_or(ExecutionError::Protocol)?;
                        if message.len() + part.len() > MAX_TEXT {
                            return Err(ExecutionError::Protocol);
                        }
                        message.push_str(part);
                    }
                }
                if !message.trim().is_empty() {
                    emit(
                        &self.events,
                        ExecutionEvent::Message {
                            provider_item: item,
                            text: message,
                        },
                    )
                    .await?;
                }
            }
            "result" => {
                if !self.requests.is_empty() {
                    return Err(ExecutionError::Protocol);
                }
                self.result = Some(
                    if frame["subtype"] == "success" && frame["is_error"] == false {
                        ExecutionEvent::Completed
                    } else if self.stopping {
                        ExecutionEvent::Uncertain
                    } else {
                        ExecutionEvent::Failed
                    },
                );
            }
            "control_request" => self.question(&frame).await?,
            "user" => self.delivered(&frame).await?,
            "control_response"
                if frame["response"]["request_id"] == "factory-interrupt" && self.stopping =>
            {
                if frame["response"]["subtype"] != "success" {
                    return Err(ExecutionError::Protocol);
                }
            }
            "rate_limit_event" | "command_lifecycle" | "stream_event" | "assistant" => {}
            _ => return Err(ExecutionError::Protocol),
        }
        Ok(())
    }

    async fn question(&mut self, frame: &Value) -> Result<(), ExecutionError> {
        if frame["request"]["subtype"] != "can_use_tool"
            || frame["request"]["tool_name"] != "AskUserQuestion"
            || self.requests.len() >= 20
        {
            return Err(ExecutionError::Protocol);
        }
        let native_id = text(&frame["request_id"], 512)?.to_owned();
        let tool_id = text(&frame["request"]["tool_use_id"], 512)?.to_owned();
        if self
            .requests
            .values()
            .any(|request| request.native_id == native_id || request.tool_id == tool_id)
        {
            return Err(ExecutionError::Protocol);
        }
        let input = frame["request"]["input"].clone();
        let native_questions = input["questions"]
            .as_array()
            .ok_or(ExecutionError::Protocol)?;
        if native_questions.is_empty() || native_questions.len() > 20 {
            return Err(ExecutionError::Protocol);
        }
        let mut questions = Vec::new();
        let mut seen = HashSet::new();
        for native in native_questions {
            let question = content_text(&native["question"], 4000)?.to_owned();
            if !seen.insert(question.clone()) {
                return Err(ExecutionError::Protocol);
            }
            let header = content_text(&native["header"], 4000)?.to_owned();
            let native_options = native["options"]
                .as_array()
                .ok_or(ExecutionError::Protocol)?;
            if native_options.len() > 20 {
                return Err(ExecutionError::Protocol);
            }
            let mut options = Vec::new();
            for option in native_options {
                options.push(QuestionOption {
                    label: text(&option["label"], 4000)?.to_owned(),
                    description: option["description"]
                        .as_str()
                        .filter(|s| s.len() <= 4000 && !s.contains('\0'))
                        .ok_or(ExecutionError::Protocol)?
                        .to_owned(),
                });
            }
            questions.push(ExecutionQuestion {
                id: Uuid::new_v4(),
                header,
                question,
                allow_free_text: true,
                options,
            });
        }
        let id = Uuid::new_v4();
        self.requests.insert(
            id,
            PendingQuestion {
                native_id,
                tool_id,
                input,
                questions: questions.clone(),
                sent: false,
            },
        );
        emit(
            &self.events,
            ExecutionEvent::Questions {
                request_id: id,
                questions,
            },
        )
        .await
    }

    async fn answer(
        &mut self,
        id: Uuid,
        answers: Vec<QuestionAnswer>,
    ) -> Result<(), ExecutionError> {
        let request = self.requests.get_mut(&id).ok_or(ExecutionError::Conflict)?;
        if request.sent {
            return Err(ExecutionError::Conflict);
        }
        validate_answers(&request.questions, &answers)?;
        let mut mapped = serde_json::Map::new();
        for question in &request.questions {
            let answer = answers
                .iter()
                .find(|answer| answer.id == question.id)
                .ok_or(ExecutionError::Invalid)?;
            mapped.insert(
                question.question.clone(),
                Value::String(answer.value.clone()),
            );
        }
        let mut input = request.input.clone();
        input
            .as_object_mut()
            .ok_or(ExecutionError::Protocol)?
            .insert("answers".into(), Value::Object(mapped));
        let response = json!({"type":"control_response","response":{"subtype":"success","request_id":request.native_id,"response":{"behavior":"allow","updatedInput":input,"toolUseID":request.tool_id}}});
        // Once submitted, no retry can safely infer whether the CLI received the answer.
        request.sent = true;
        self.stream.send(response).await
    }

    async fn delivered(&mut self, frame: &Value) -> Result<(), ExecutionError> {
        let Some(blocks) = frame["message"]["content"].as_array() else {
            return Ok(());
        };
        for block in blocks {
            if block["type"] != "tool_result" {
                continue;
            }
            let Some(native) = block["tool_use_id"].as_str() else {
                return Err(ExecutionError::Protocol);
            };
            let id = self.requests.iter().find_map(|(id, request)| {
                (request.sent && request.tool_id == native).then_some(*id)
            });
            if let Some(id) = id {
                if block["is_error"] == true {
                    return Err(ExecutionError::Protocol);
                }
                self.requests.remove(&id);
                emit(
                    &self.events,
                    ExecutionEvent::ResponseDelivered { request_id: id },
                )
                .await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::container::LogOutput;
    use futures_util::stream;
    use tokio::io::{AsyncBufReadExt, BufReader};

    async fn read_request(input: &mut BufReader<tokio::io::DuplexStream>) -> Value {
        let mut line = String::new();
        assert_ne!(input.read_line(&mut line).await.unwrap(), 0);
        serde_json::from_str(&line).unwrap()
    }
    async fn frame(tx: &mpsc::Sender<Result<LogOutput, bollard::errors::Error>>, value: Value) {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        tx.send(Ok(LogOutput::StdOut {
            message: bytes.into(),
        }))
        .await
        .unwrap();
    }

    #[test]
    fn catalog_rejects_invalid_models_and_deduplicates() {
        let models = parse_models(&json!({"models":[{"value":"sonnet","displayName":"Sonnet"},{"value":"sonnet","displayName":"Duplicate"},{"value":"opus[1m]","displayName":"Opus","resolvedModel":"claude-opus-5-5","supportedEffortLevels":["low","high","max"]}]})).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[1].id, "opus[1m]");
        assert_eq!(models[1].resolved_model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(models[1].effort_levels, ["low", "high", "max"]);
        assert!(parse_models(&json!({"models":[{"value":"opus","displayName":"Opus","supportedEffortLevels":["--bad"]}]})).is_err());
        assert!(
            parse_models(&json!({"models":[{"value":"--help","displayName":"Wrong"}]})).is_err()
        );
        assert!(parse_models(&json!({"models":[]})).is_err());
    }

    #[tokio::test]
    async fn streams_messages_and_waits_for_background_completion() {
        let session = Uuid::new_v4();
        let (client, server) = tokio::io::duplex(65536);
        let (output_tx, output_rx) = mpsc::channel(32);
        let output = stream::unfold(output_rx, |mut rx| async move {
            rx.recv().await.map(|v| (v, rx))
        });
        let (command_tx, command_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let mut control = Control {
            commands: command_tx,
        };
        let worker = tokio::spawn(
            Worker::new(
                JsonStream::new(Box::pin(client), Box::pin(output)),
                command_rx,
                event_tx,
                session,
                "hello".into(),
            )
            .run(),
        );
        let (release, wait) = oneshot::channel::<()>();
        let peer = tokio::spawn(async move {
            let mut input = BufReader::new(server);
            let init = read_request(&mut input).await;
            assert_eq!(init["request"]["subtype"], "initialize");
            frame(&output_tx, json!({"type":"control_response","response":{"request_id":"factory-init","subtype":"success","response":{}}})).await;
            let prompt = read_request(&mut input).await;
            assert_eq!(prompt["message"]["content"], "hello");
            frame(&output_tx, json!({"type":"stream_event","session_id":session.to_string(),"parent_tool_use_id":null,"event":{"type":"message_start","message":{"id":"message"}}})).await;
            frame(&output_tx, json!({"type":"command_lifecycle","session_id":session.to_string(),"command_uuid":"command","state":"started"})).await;
            frame(&output_tx, json!({"type":"stream_event","session_id":session.to_string(),"parent_tool_use_id":null,"event":{"type":"content_block_delta","delta":{"type":"text_delta","text":" "}}})).await;
            frame(&output_tx, json!({"type":"assistant","uuid":"frame-1","session_id":session.to_string(),"parent_tool_use_id":null,"message":{"id":"message","content":[{"type":"text","text":"hello"}]}})).await;
            frame(&output_tx, json!({"type":"system","subtype":"task_started","session_id":session.to_string(),"task_id":"child"})).await;
            frame(
                &output_tx,
                json!({"type":"result","subtype":"success","is_error":false,"session_id":session.to_string()}),
            )
            .await;
            wait.await.unwrap();
            frame(&output_tx, json!({"type":"system","subtype":"task_notification","session_id":session.to_string(),"task_id":"child"})).await;
        });
        assert!(matches!(
            event_rx.recv().await,
            Some(ExecutionEvent::ThreadReady { .. })
        ));
        control.begin_turn().await.unwrap();
        assert!(
            matches!(event_rx.recv().await, Some(ExecutionEvent::Delta { text, .. }) if text == " ")
        );
        assert!(
            matches!(event_rx.recv().await, Some(ExecutionEvent::Message { provider_item, text }) if provider_item == "frame-1" && text == "hello")
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(40), event_rx.recv())
                .await
                .is_err()
        );
        release.send(()).unwrap();
        let (outcome, _stream) = worker.await.unwrap();
        assert_eq!(outcome.unwrap(), ExecutionEvent::Completed);
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn answers_require_a_correlated_tool_result_before_delivery() {
        let session = Uuid::new_v4();
        let (client, server) = tokio::io::duplex(65536);
        let (output_tx, output_rx) = mpsc::channel(32);
        let output = stream::unfold(output_rx, |mut rx| async move {
            rx.recv().await.map(|v| (v, rx))
        });
        let (command_tx, command_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let mut control = Control {
            commands: command_tx,
        };
        let worker = tokio::spawn(
            Worker::new(
                JsonStream::new(Box::pin(client), Box::pin(output)),
                command_rx,
                event_tx,
                session,
                "hello".into(),
            )
            .run(),
        );
        let (release, wait) = oneshot::channel::<()>();
        let peer = tokio::spawn(async move {
            let mut input = BufReader::new(server);
            let _ = read_request(&mut input).await;
            frame(&output_tx, json!({"type":"control_response","response":{"request_id":"factory-init","subtype":"success","response":{}}})).await;
            let _ = read_request(&mut input).await;
            frame(&output_tx, json!({"type":"control_request","request_id":"native-request","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","tool_use_id":"native-tool","input":{"questions":[{"header":"Choice","question":"Which?","options":[{"label":"A","description":"First"}]}]}}})).await;
            let answer = read_request(&mut input).await;
            assert_eq!(answer["response"]["request_id"], "native-request");
            assert_eq!(
                answer["response"]["response"]["updatedInput"]["answers"]["Which?"],
                "A"
            );
            wait.await.unwrap();
            frame(&output_tx, json!({"type":"user","session_id":session.to_string(),"message":{"content":[{"type":"tool_result","tool_use_id":"native-tool","content":"accepted"}]}})).await;
            frame(&output_tx, json!({"type":"result","subtype":"success","is_error":false,"session_id":session.to_string()})).await;
        });
        assert!(matches!(
            event_rx.recv().await,
            Some(ExecutionEvent::ThreadReady { .. })
        ));
        control.begin_turn().await.unwrap();
        let Some(ExecutionEvent::Questions {
            request_id,
            questions,
        }) = event_rx.recv().await
        else {
            panic!("expected question")
        };
        control
            .answer(
                request_id,
                vec![QuestionAnswer {
                    id: questions[0].id,
                    value: "A".into(),
                }],
            )
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(40), event_rx.recv())
                .await
                .is_err()
        );
        release.send(()).unwrap();
        assert_eq!(
            event_rx.recv().await,
            Some(ExecutionEvent::ResponseDelivered { request_id })
        );
        let (outcome, _stream) = worker.await.unwrap();
        assert_eq!(outcome.unwrap(), ExecutionEvent::Completed);
        peer.await.unwrap();
    }

    #[tokio::test]
    async fn interrupt_acknowledgement_does_not_prove_the_run_stopped() {
        let session = Uuid::new_v4();
        let (client, server) = tokio::io::duplex(65536);
        let (output_tx, output_rx) = mpsc::channel(32);
        let output = stream::unfold(output_rx, |mut rx| async move {
            rx.recv().await.map(|v| (v, rx))
        });
        let (command_tx, command_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let mut control = Control {
            commands: command_tx,
        };
        let worker = tokio::spawn(
            Worker::new(
                JsonStream::new(Box::pin(client), Box::pin(output)),
                command_rx,
                event_tx,
                session,
                "hello".into(),
            )
            .run(),
        );
        let (release, wait) = oneshot::channel::<()>();
        let peer = tokio::spawn(async move {
            let mut input = BufReader::new(server);
            let _ = read_request(&mut input).await;
            frame(&output_tx, json!({"type":"control_response","response":{"request_id":"factory-init","subtype":"success","response":{}}})).await;
            let _ = read_request(&mut input).await;
            let interrupt = read_request(&mut input).await;
            assert_eq!(interrupt["request"]["subtype"], "interrupt");
            frame(&output_tx, json!({"type":"control_response","response":{"request_id":"factory-interrupt","subtype":"success","response":{}}})).await;
            wait.await.unwrap();
        });
        assert!(matches!(
            event_rx.recv().await,
            Some(ExecutionEvent::ThreadReady { .. })
        ));
        control.begin_turn().await.unwrap();
        control.interrupt().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(40), event_rx.recv())
                .await
                .is_err()
        );
        release.send(()).unwrap();
        let (outcome, _stream) = worker.await.unwrap();
        assert!(outcome.is_err());
        peer.await.unwrap();
    }
}
