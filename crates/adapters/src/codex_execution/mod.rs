//! Pinned Codex app-server inside an owned environment. HTTP and persistence remain outside.
mod transport;
use crate::environments::{
    DockerEnvironmentRuntime,
    process::{DockerProcess, FixedProcess, ProcessStartError},
};
use async_trait::async_trait;
use factory_core::{conversations::execution::*, environments::Environment};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
use transport::Transport;
use uuid::Uuid;

const LIMIT: usize = 64;
const MAX_TEXT: usize = 256 * 1024;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const EMIT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct CodexExecutionAdapter {
    runtime: Arc<DockerEnvironmentRuntime>,
}
impl CodexExecutionAdapter {
    pub fn new(runtime: Arc<DockerEnvironmentRuntime>) -> Self {
        Self { runtime }
    }
    async fn process(&self, env: &Environment) -> Result<(String, Transport), ExecutionError> {
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
            .start_process_checked(env, FixedProcess::CodexExecution)
            .await
            .map_err(|error| match error {
                ProcessStartError::PreSubmission(_) => ExecutionError::PreflightUnavailable,
                ProcessStartError::Submitted(_) => ExecutionError::LaunchUncertain,
            })?;
        Ok((id, Transport::new(input, output)))
    }
}
#[async_trait]
impl ExecutionRuntime for CodexExecutionAdapter {
    async fn models(&self, env: &Environment) -> Result<Vec<ExecutionModel>, ExecutionError> {
        let (id, mut transport) = self.process(env).await?;
        let result = async {
            transport.initialize().await?;
            let mut cursor: Option<String> = None;
            let mut models = Vec::new();
            let mut seen = HashSet::new();
            for _ in 0..10 {
                let page = transport
                    .request("model/list", json!({"cursor":cursor,"limit":100}))
                    .await?;
                let items = page["data"].as_array().ok_or(ExecutionError::Protocol)?;
                for item in items {
                    let id = text(&item["model"], 512)?;
                    let name = text(&item["displayName"], 512)?;
                    if seen.insert(id.clone()) {
                        let levels = item["supportedReasoningEfforts"]
                            .as_array()
                            .ok_or(ExecutionError::Protocol)?;
                        if levels.len() > 20 {
                            return Err(ExecutionError::Protocol);
                        }
                        let effort_levels = levels
                            .iter()
                            .map(|level| {
                                let value = text(&level["reasoningEffort"], 32)?;
                                if valid_effort(&value) {
                                    Ok(value)
                                } else {
                                    Err(ExecutionError::Protocol)
                                }
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let default_effort = if item["defaultReasoningEffort"].is_null() {
                            None
                        } else {
                            let value = text(&item["defaultReasoningEffort"], 32)?;
                            if !effort_levels.contains(&value) {
                                return Err(ExecutionError::Protocol);
                            }
                            Some(value)
                        };
                        models.push(ExecutionModel {
                            resolved_model: Some(id.clone()),
                            id,
                            name,
                            effort_levels,
                            default_effort,
                        });
                    }
                    if models.len() > 1000 {
                        return Err(ExecutionError::Protocol);
                    }
                }
                match &page["nextCursor"] {
                    Value::Null => return Ok(models),
                    value => {
                        let next = text(value, 1024)?;
                        if cursor.as_ref() == Some(&next) {
                            return Err(ExecutionError::Protocol);
                        }
                        cursor = Some(next);
                    }
                }
            }
            Err(ExecutionError::Protocol)
        }
        .await;
        let close = transport.close().await;
        let exit = self
            .runtime
            .wait_process_exit(&id, Duration::from_secs(15))
            .await
            .map_err(|_| ExecutionError::CleanupUncertain)?;
        if close.is_err() {
            return Err(ExecutionError::CleanupUncertain);
        }
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
        if !valid(model, 512)
            || !valid(input, MAX_TEXT)
            || provider_thread.is_some_and(|id| !valid(id, 512))
            || effort.is_some_and(|value| !valid_effort(value))
        {
            return Err(ExecutionError::Invalid);
        }
        let (process_id, transport) = self.process(env).await?;
        let (commands, receiver) = mpsc::channel(LIMIT);
        let (sender, events) = mpsc::channel(LIMIT);
        let runtime = self.runtime.clone();
        let model = model.to_owned();
        let effort = effort.map(str::to_owned);
        let input = input.to_owned();
        let thread = provider_thread.map(str::to_owned);
        tokio::spawn(async move {
            let result = Worker::new(transport, receiver, sender.clone(), model, effort, input)
                .run(thread)
                .await;
            // Process exit is necessary, but never by itself proves descendant cleanup.
            let exited = runtime
                .wait_process_exit(&process_id, Duration::from_secs(15))
                .await;
            let terminal = match (result, exited) {
                (Ok(event), Ok(0)) => event,
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
fn valid(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.contains('\0')
}
fn text(value: &Value, max: usize) -> Result<String, ExecutionError> {
    let s = value.as_str().ok_or(ExecutionError::Protocol)?;
    if !valid(s, max) {
        return Err(ExecutionError::Protocol);
    }
    Ok(s.to_owned())
}
async fn emit(
    tx: &mpsc::Sender<ExecutionEvent>,
    event: ExecutionEvent,
) -> Result<(), ExecutionError> {
    tokio::time::timeout(EMIT_TIMEOUT, tx.send(event))
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
    async fn call(&mut self, make: impl FnOnce(Reply) -> Command) -> Result<(), ExecutionError> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(make(tx))
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
        self.call(|r| Command::Answer(id, answers, r)).await
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
struct Slot {
    native: String,
    question: ExecutionQuestion,
}
struct Request {
    native: Value,
    slots: Vec<Slot>,
    sent: bool,
    sent_at: Option<tokio::time::Instant>,
}
struct Worker {
    transport: Transport,
    commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<ExecutionEvent>,
    model: String,
    effort: Option<String>,
    input: String,
    thread: String,
    turn: Option<String>,
    requests: HashMap<Uuid, Request>,
    children: HashMap<String, Child>,
    outcome: Option<ExecutionEvent>,
    stopping: bool,
}
struct Child {
    parent: String,
    done: bool,
    turn: Option<String>,
    interrupt_sent: bool,
}
impl Worker {
    fn new(
        transport: Transport,
        commands: mpsc::Receiver<Command>,
        events: mpsc::Sender<ExecutionEvent>,
        model: String,
        effort: Option<String>,
        input: String,
    ) -> Self {
        Self {
            transport,
            commands,
            events,
            model,
            effort,
            input,
            thread: String::new(),
            turn: None,
            requests: HashMap::new(),
            children: HashMap::new(),
            outcome: None,
            stopping: false,
        }
    }
    async fn run(mut self, previous: Option<String>) -> Result<ExecutionEvent, ExecutionError> {
        self.transport.initialize().await?;
        let requested_thread = previous.clone();
        let (method, params) = if let Some(id) = previous {
            (
                "thread/resume",
                json!({"threadId":id,"cwd":"/home/factory/Documents"}),
            )
        } else {
            ("thread/start", json!({"cwd":"/home/factory/Documents"}))
        };
        let started = self.transport.request(method, params).await?;
        self.thread = text(&started["thread"]["id"], 512)?;
        if requested_thread
            .as_ref()
            .is_some_and(|id| id != &self.thread)
        {
            return Err(ExecutionError::Protocol);
        }
        emit(
            &self.events,
            ExecutionEvent::ThreadReady {
                provider_thread: self.thread.clone(),
            },
        )
        .await?;
        let mut deadline: Option<tokio::time::Instant> = None;
        loop {
            if self.outcome.is_some()
                && self.children.values().all(|child| child.done)
                && self.requests.is_empty()
            {
                let event = self.outcome.take().ok_or(ExecutionError::Protocol)?;
                self.transport.close().await?;
                return Ok(event);
            }
            if self.outcome.is_some()
                && self.children.values().any(|child| !child.done)
                && deadline.is_none()
            {
                deadline = Some(tokio::time::Instant::now() + RESPONSE_TIMEOUT);
            }
            tokio::select! {
                command=self.commands.recv() => {
                    let command=command.ok_or(ExecutionError::Unavailable)?;
                    match command {
                        Command::Begin(reply) => {
                            let result=self.begin().await; let failed=result.is_err(); let _=reply.send(result);
                            if failed { return Err(ExecutionError::Protocol); }
                        }
                        Command::Answer(id,answers,reply) => {
                            let result=self.answer(id,answers).await; let failed=matches!(result,Err(ExecutionError::Unavailable|ExecutionError::Protocol)); let _=reply.send(result);
                            if failed { return Err(ExecutionError::Protocol); }
                        }
                        Command::Interrupt(reply) => {
                            let result=self.interrupt().await; let failed=result.is_err(); let _=reply.send(result);
                            if failed { return Err(ExecutionError::CleanupUncertain); }
                            deadline=Some(tokio::time::Instant::now()+RESPONSE_TIMEOUT);
                        }
                    }
                }
                frame=self.transport.next() => { self.frame(frame?).await?; }
                _=tokio::time::sleep(Duration::from_millis(250)) => {
                    let now=tokio::time::Instant::now();
                    if deadline.is_some_and(|d| now>=d) || self.requests.values().any(|r| r.sent_at.is_some_and(|t| now.duration_since(t)>=RESPONSE_TIMEOUT)) { return Err(ExecutionError::CleanupUncertain); }
                }
            }
        }
    }
    async fn begin(&mut self) -> Result<(), ExecutionError> {
        if self.turn.is_some() || self.outcome.is_some() {
            return Err(ExecutionError::Conflict);
        }
        let result = self
            .transport
            .request(
                "turn/start",
                turn_start(
                    &self.thread,
                    &self.model,
                    self.effort.as_deref(),
                    &self.input,
                ),
            )
            .await?;
        self.turn = Some(text(&result["turn"]["id"], 512)?);
        Ok(())
    }
    async fn interrupt(&mut self) -> Result<(), ExecutionError> {
        let turn = self.turn.as_ref().ok_or(ExecutionError::Conflict)?;
        if self.stopping {
            return Err(ExecutionError::Conflict);
        }
        self.stopping = true;
        self.transport
            .request(
                "turn/interrupt",
                json!({"threadId":self.thread,"turnId":turn}),
            )
            .await?;
        let children: Vec<_> = self
            .children
            .iter()
            .filter_map(|(thread, child)| {
                (!child.done && !child.interrupt_sent)
                    .then(|| {
                        child
                            .turn
                            .as_ref()
                            .map(|turn| (thread.clone(), turn.clone()))
                    })
                    .flatten()
            })
            .collect();
        for (thread, turn) in children {
            self.interrupt_child(&thread, &turn).await?;
        }
        Ok(())
    }

    async fn interrupt_child(&mut self, thread: &str, turn: &str) -> Result<(), ExecutionError> {
        let child = self
            .children
            .get_mut(thread)
            .ok_or(ExecutionError::Protocol)?;
        if child.interrupt_sent {
            return Ok(());
        }
        child.interrupt_sent = true;
        tokio::time::timeout(
            Duration::from_secs(3),
            self.transport
                .request("turn/interrupt", json!({"threadId":thread,"turnId":turn})),
        )
        .await
        .map_err(|_| ExecutionError::CleanupUncertain)??;
        Ok(())
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
        let questions: Vec<_> = request.slots.iter().map(|s| s.question.clone()).collect();
        validate_answers(&questions, &answers)?;
        let mut output = serde_json::Map::new();
        for slot in &request.slots {
            let mut candidates = answers.iter().filter(|a| a.id == slot.question.id);
            let answer = candidates.next().ok_or(ExecutionError::Invalid)?;
            if candidates.next().is_some() {
                return Err(ExecutionError::Invalid);
            }
            output.insert(slot.native.clone(), json!({"answers":[answer.value]}));
        }
        request.sent = true;
        request.sent_at = Some(tokio::time::Instant::now());
        self.transport
            .send(json!({"id":request.native,"result":{"answers":output}}))
            .await
    }
    async fn frame(&mut self, frame: Value) -> Result<(), ExecutionError> {
        let method = text(&frame["method"], 256)?;
        let params = &frame["params"];
        if frame.get("id").is_some() {
            return self.server_request(&frame).await;
        }
        if method == "serverRequest/resolved" {
            let thread = text(&params["threadId"], 512)?;
            if thread != self.thread {
                return Err(ExecutionError::Protocol);
            }
            let native = &params["requestId"];
            let id = self
                .requests
                .iter()
                .find_map(|(id, r)| (r.native == *native && r.sent).then_some(*id))
                .ok_or(ExecutionError::Protocol)?;
            self.requests.remove(&id);
            return emit(
                &self.events,
                ExecutionEvent::ResponseDelivered { request_id: id },
            )
            .await;
        }
        if method == "turn/started" {
            let thread = text(&params["threadId"], 512)?;
            let turn = text(&params["turn"]["id"], 512)?;
            if thread == self.thread {
                return Ok(());
            }
            let child = self
                .children
                .get_mut(&thread)
                .ok_or(ExecutionError::Protocol)?;
            child.turn = Some(turn.clone());
            child.done = false;
            child.interrupt_sent = false;
            if self.stopping {
                self.interrupt_child(&thread, &turn).await?;
            }
            return Ok(());
        }
        if matches!(
            method.as_str(),
            "item/agentMessage/delta" | "item/started" | "item/completed" | "turn/completed"
        ) {
            let thread = text(&params["threadId"], 512)?;
            if thread != self.thread && !self.children.contains_key(&thread) {
                return Err(ExecutionError::Protocol);
            }
            if method == "turn/completed" {
                if thread != self.thread {
                    match params["turn"]["status"].as_str() {
                        Some("completed" | "failed" | "interrupted") => {
                            self.children
                                .get_mut(&thread)
                                .ok_or(ExecutionError::Protocol)?
                                .done = true;
                            return Ok(());
                        }
                        _ => return Err(ExecutionError::Protocol),
                    }
                }
                if self.turn.as_deref() != params["turn"]["id"].as_str() {
                    return Err(ExecutionError::Protocol);
                }
                self.outcome = Some(match params["turn"]["status"].as_str() {
                    Some("completed") => ExecutionEvent::Completed,
                    Some("failed") => ExecutionEvent::Failed,
                    Some("interrupted") => ExecutionEvent::Interrupted,
                    _ => return Err(ExecutionError::Protocol),
                });
                return Ok(());
            }
            if thread == self.thread && self.turn.as_deref() != params["turnId"].as_str() {
                return Err(ExecutionError::Protocol);
            }
            if method == "item/agentMessage/delta" {
                if thread != self.thread {
                    return Ok(());
                }
                let item = text(&params["itemId"], 512)?;
                let delta = params["delta"].as_str().ok_or(ExecutionError::Protocol)?;
                if delta.len() > MAX_TEXT {
                    return Err(ExecutionError::Protocol);
                }
                return emit(
                    &self.events,
                    ExecutionEvent::Delta {
                        provider_item: item,
                        text: delta.to_owned(),
                    },
                )
                .await;
            }
            let item = &params["item"];
            match item["type"].as_str() {
                Some("agentMessage") if method == "item/completed" && thread == self.thread => {
                    let id = text(&item["id"], 512)?;
                    let message = item["text"].as_str().ok_or(ExecutionError::Protocol)?;
                    if message.len() > MAX_TEXT {
                        return Err(ExecutionError::Protocol);
                    }
                    return emit(
                        &self.events,
                        ExecutionEvent::Message {
                            provider_item: id,
                            text: message.to_owned(),
                        },
                    )
                    .await;
                }
                Some("collabAgentToolCall") => self.observe_children(&thread, item)?,
                Some(_) => (),
                None => return Err(ExecutionError::Protocol),
            }
        }
        if method == "error" || method == "thread/closed" {
            return Err(ExecutionError::Unavailable);
        }
        Ok(())
    }
    fn observe_children(&mut self, parent: &str, item: &Value) -> Result<(), ExecutionError> {
        if item["senderThreadId"].as_str() != Some(parent) {
            return Err(ExecutionError::Protocol);
        }
        if item["tool"].as_str() == Some("spawnAgent") {
            let children = item["receiverThreadIds"]
                .as_array()
                .ok_or(ExecutionError::Protocol)?;
            for child in children {
                let id = text(child, 512)?;
                if id == self.thread || self.children.len() >= LIMIT {
                    return Err(ExecutionError::Protocol);
                }
                match self.children.get_mut(&id) {
                    Some(existing) if existing.parent != parent => {
                        return Err(ExecutionError::Protocol);
                    }
                    Some(_) => (),
                    None => {
                        self.children.insert(
                            id,
                            Child {
                                parent: parent.to_owned(),
                                done: false,
                                turn: None,
                                interrupt_sent: false,
                            },
                        );
                    }
                }
            }
        }
        let states = item["agentsStates"]
            .as_object()
            .ok_or(ExecutionError::Protocol)?;
        for (id, state) in states {
            if let Some(child) = self.children.get_mut(id) {
                if child.done && matches!(state["status"].as_str(), Some("pendingInit" | "running"))
                {
                    child.turn = None;
                    child.interrupt_sent = false;
                }
                child.done = match state["status"].as_str() {
                    Some("pendingInit" | "running") => false,
                    Some("interrupted" | "completed" | "errored" | "shutdown" | "notFound") => true,
                    _ => return Err(ExecutionError::Protocol),
                };
            } else {
                return Err(ExecutionError::Protocol);
            }
        }
        Ok(())
    }
    async fn server_request(&mut self, frame: &Value) -> Result<(), ExecutionError> {
        if frame["method"] != "item/tool/requestUserInput" || self.requests.len() >= LIMIT {
            return Err(ExecutionError::Protocol);
        }
        let params = &frame["params"];
        if params["threadId"].as_str() != Some(&self.thread)
            || params["turnId"].as_str() != self.turn.as_deref()
        {
            return Err(ExecutionError::Protocol);
        }
        let native = frame["id"].clone();
        if !native.is_string() && !native.is_i64() && !native.is_u64() {
            return Err(ExecutionError::Protocol);
        }
        if self.requests.values().any(|r| r.native == native) {
            return Err(ExecutionError::Protocol);
        }
        let data = params["questions"]
            .as_array()
            .ok_or(ExecutionError::Protocol)?;
        if data.is_empty() || data.len() > 20 {
            return Err(ExecutionError::Protocol);
        }
        let mut slots = Vec::new();
        let mut seen = HashSet::new();
        for q in data {
            if q["isSecret"] != false {
                return Err(ExecutionError::Protocol);
            }
            let provider = text(&q["id"], 512)?;
            if !seen.insert(provider.clone()) {
                return Err(ExecutionError::Protocol);
            }
            let mut options = Vec::new();
            if !q["options"].is_null() {
                let entries = q["options"].as_array().ok_or(ExecutionError::Protocol)?;
                if entries.len() > 20 {
                    return Err(ExecutionError::Protocol);
                }
                for o in entries {
                    options.push(QuestionOption {
                        label: text(&o["label"], 4000)?,
                        description: o["description"]
                            .as_str()
                            .ok_or(ExecutionError::Protocol)?
                            .chars()
                            .take(4000)
                            .collect(),
                    });
                }
            }
            let question = ExecutionQuestion {
                id: Uuid::new_v4(),
                header: q["header"]
                    .as_str()
                    .ok_or(ExecutionError::Protocol)?
                    .chars()
                    .take(4000)
                    .collect(),
                question: text(&q["question"], 4000)?,
                allow_free_text: q["isOther"].as_bool().ok_or(ExecutionError::Protocol)?
                    || options.is_empty(),
                options,
            };
            slots.push(Slot {
                native: provider,
                question,
            });
        }
        let id = Uuid::new_v4();
        let questions = slots.iter().map(|s| s.question.clone()).collect();
        self.requests.insert(
            id,
            Request {
                native,
                slots,
                sent: false,
                sent_at: None,
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
}
fn turn_start(thread: &str, model: &str, effort: Option<&str>, input: &str) -> Value {
    let mut result = json!({"threadId":thread,"model":model,"approvalPolicy":"never","sandboxPolicy":{"type":"dangerFullAccess"},"input":[{"type":"text","text":input,"text_elements":[]}]});
    if let Some(effort) = effort {
        result["effort"] = json!(effort);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_full_access_is_attached_to_each_turn() {
        let turn = turn_start("thread", "model", Some("high"), "hello");
        assert_eq!(turn["effort"], "high");
        assert_eq!(turn["approvalPolicy"], "never");
        assert_eq!(turn["sandboxPolicy"]["type"], "dangerFullAccess");
        assert_eq!(turn["threadId"], "thread");
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use bollard::container::LogOutput;
    use futures_util::stream;
    use tokio::io::{AsyncBufReadExt, BufReader};

    async fn read_request(input: &mut BufReader<tokio::io::DuplexStream>) -> Value {
        let mut line = String::new();
        assert_ne!(input.read_line(&mut line).await.unwrap(), 0);
        serde_json::from_str(&line).unwrap()
    }
    async fn write_frame(
        tx: &mpsc::Sender<Result<LogOutput, bollard::errors::Error>>,
        value: Value,
    ) {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        tx.send(Ok(LogOutput::StdOut {
            message: bytes.into(),
        }))
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn observed_child_stays_active_until_its_terminal_event() {
        let output = stream::empty::<Result<LogOutput, bollard::errors::Error>>();
        let transport = Transport::new(Box::pin(tokio::io::sink()), Box::pin(output));
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut worker = Worker::new(
            transport,
            command_rx,
            event_tx,
            "model".into(),
            None,
            "input".into(),
        );
        worker.thread = "root".into();
        worker.turn = Some("turn".into());
        worker.frame(json!({"method":"item/started","params":{"threadId":"root","turnId":"turn","item":{"type":"collabAgentToolCall","senderThreadId":"root","tool":"spawnAgent","receiverThreadIds":["child"],"agentsStates":{"child":{"status":"running"}}}}})).await.unwrap();
        worker.frame(json!({"method":"turn/completed","params":{"threadId":"root","turn":{"id":"turn","status":"completed"}}})).await.unwrap();
        assert!(worker.outcome.is_some());
        assert!(!worker.children["child"].done);
        worker.frame(json!({"method":"turn/completed","params":{"threadId":"child","turn":{"id":"child-turn","status":"interrupted"}}})).await.unwrap();
        assert!(worker.children["child"].done);
        assert!(worker.observe_children("wrong-parent", &json!({"senderThreadId":"root","tool":"spawnAgent","receiverThreadIds":["new"],"agentsStates":{}})).is_err());
    }
    #[tokio::test]
    async fn multiplexes_question_response_and_waits_for_correlated_receipt() {
        let (client, server) = tokio::io::duplex(65536);
        let (output_tx, output_rx) = mpsc::channel(32);
        let output = stream::unfold(output_rx, |mut rx| async move {
            rx.recv().await.map(|v| (v, rx))
        });
        let transport = Transport::new(Box::pin(client), Box::pin(output));
        let (command_tx, command_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let mut control = Control {
            commands: command_tx,
        };
        let worker = tokio::spawn(
            Worker::new(
                transport,
                command_rx,
                event_tx,
                "chosen-model".into(),
                Some("high".into()),
                "hello".into(),
            )
            .run(None),
        );
        let (release, wait) = oneshot::channel::<()>();
        let peer = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            let init = read_request(&mut server).await;
            assert_eq!(init["method"], "initialize");
            write_frame(
                &output_tx,
                json!({"id":init["id"],"result":{"userAgent":"fixture"}}),
            )
            .await;
            assert_eq!(read_request(&mut server).await["method"], "initialized");
            let start = read_request(&mut server).await;
            assert_eq!(start["method"], "thread/start");
            write_frame(
                &output_tx,
                json!({"id":start["id"],"result":{"thread":{"id":"thread-1"}}}),
            )
            .await;
            let turn = read_request(&mut server).await;
            assert_eq!(turn["params"]["model"], "chosen-model");
            assert_eq!(turn["params"]["effort"], "high");
            assert_eq!(turn["params"]["approvalPolicy"], "never");
            assert_eq!(turn["params"]["sandboxPolicy"]["type"], "dangerFullAccess");
            // An event can precede the correlated turn/start response.
            write_frame(&output_tx,json!({"method":"item/agentMessage/delta","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"item-1","delta":"hi"}})).await;
            write_frame(
                &output_tx,
                json!({"id":turn["id"],"result":{"turn":{"id":"turn-1"}}}),
            )
            .await;
            write_frame(&output_tx,json!({"id":42,"method":"item/tool/requestUserInput","params":{"threadId":"thread-1","turnId":"turn-1","questions":[{"id":"native-1","header":"H","question":"Choose","isSecret":false,"isOther":false,"options":[{"label":"A","description":"one"}]},{"id":"native-2","header":"H2","question":"Why","isSecret":false,"isOther":true,"options":null}]}})).await;
            let answer = read_request(&mut server).await;
            assert_eq!(answer["id"], 42);
            assert_eq!(answer["result"]["answers"]["native-1"]["answers"][0], "A");
            assert_eq!(
                answer["result"]["answers"]["native-2"]["answers"][0],
                "because"
            );
            wait.await.unwrap();
            write_frame(&output_tx,json!({"method":"serverRequest/resolved","params":{"threadId":"thread-1","requestId":42}})).await;
            write_frame(&output_tx,json!({"method":"item/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":{"id":"item-1","type":"agentMessage","text":"hi"}}})).await;
            write_frame(&output_tx,json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}})).await;
        });
        assert!(matches!(
            event_rx.recv().await,
            Some(ExecutionEvent::ThreadReady { .. })
        ));
        control.begin_turn().await.unwrap();
        assert!(matches!(
            event_rx.recv().await,
            Some(ExecutionEvent::Delta { .. })
        ));
        let Some(ExecutionEvent::Questions {
            request_id,
            questions,
        }) = event_rx.recv().await
        else {
            panic!("expected question")
        };
        assert_eq!(questions.len(), 2);
        assert!(
            control
                .answer(
                    request_id,
                    vec![
                        QuestionAnswer {
                            id: questions[0].id,
                            value: "invalid".into()
                        },
                        QuestionAnswer {
                            id: questions[1].id,
                            value: "because".into()
                        }
                    ]
                )
                .await
                .is_err()
        );
        control
            .answer(
                request_id,
                vec![
                    QuestionAnswer {
                        id: questions[0].id,
                        value: "A".into(),
                    },
                    QuestionAnswer {
                        id: questions[1].id,
                        value: "because".into(),
                    },
                ],
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
        assert!(matches!(
            event_rx.recv().await,
            Some(ExecutionEvent::Message { .. })
        ));
        assert_eq!(worker.await.unwrap().unwrap(), ExecutionEvent::Completed);
        peer.await.unwrap();
    }
    #[tokio::test]
    async fn interrupt_ack_does_not_confirm_stopped_turn() {
        let (client, server) = tokio::io::duplex(65536);
        let (output_tx, output_rx) = mpsc::channel(32);
        let output = stream::unfold(output_rx, |mut rx| async move {
            rx.recv().await.map(|v| (v, rx))
        });
        let transport = Transport::new(Box::pin(client), Box::pin(output));
        let (command_tx, command_rx) = mpsc::channel(16);
        let (event_tx, mut event_rx) = mpsc::channel(16);
        let mut control = Control {
            commands: command_tx,
        };
        let worker = tokio::spawn(
            Worker::new(
                transport,
                command_rx,
                event_tx,
                "model".into(),
                None,
                "hello".into(),
            )
            .run(None),
        );
        let (release, wait) = oneshot::channel::<()>();
        let peer = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            let init = read_request(&mut server).await;
            write_frame(
                &output_tx,
                json!({"id":init["id"],"result":{"userAgent":"fixture"}}),
            )
            .await;
            let _ = read_request(&mut server).await;
            let start = read_request(&mut server).await;
            write_frame(
                &output_tx,
                json!({"id":start["id"],"result":{"thread":{"id":"thread-1"}}}),
            )
            .await;
            let turn = read_request(&mut server).await;
            write_frame(
                &output_tx,
                json!({"id":turn["id"],"result":{"turn":{"id":"turn-1"}}}),
            )
            .await;
            let interrupt = read_request(&mut server).await;
            assert_eq!(interrupt["method"], "turn/interrupt");
            write_frame(&output_tx, json!({"id":interrupt["id"],"result":{}})).await;
            wait.await.unwrap();
            // A lost transport after the acknowledgement is not completion.
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
        assert!(worker.await.unwrap().is_err());
        peer.await.unwrap();
    }
}
