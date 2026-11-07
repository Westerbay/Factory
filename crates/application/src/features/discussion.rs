use super::{
    FeatureCoordinator, bundle, conversation_error,
    coordinator::{ReservedTask, TaskRequest, TaskResult},
    workspace_error,
};
use factory_core::{
    conversations::{
        ConversationError, ConversationHistory, MessageRole, Run, RunStatus,
        execution::{ExecutionError, ExecutionSession},
    },
    environments::{Environment, EnvironmentActor},
    features::*,
    projects::Project,
};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

const MARKER: &str = "```factory-proposal";
const PROMPT_LIMIT: usize = 240_000;

impl FeatureCoordinator {
    pub async fn discuss(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        id: Uuid,
        operation_id: Uuid,
        input: String,
    ) -> Result<Run, FeatureError> {
        if operation_id.is_nil()
            || input.trim().is_empty()
            || input.len() > 65_536
            || input.contains('\0')
        {
            return Err(FeatureError::Invalid);
        }
        let (env, project, state) = self.context(actor, id).await?;
        let conversation = state.feature.conversation_id;
        // Replays return the original run even when its reply advanced the gate.
        if let Some(attempt) = state
            .attempts
            .iter()
            .find(|attempt| attempt.id() == operation_id)
        {
            let model = format!("factory:{}:{}", state.feature.provider, attempt.claim.model);
            let selection = factory_core::conversations::execution::encode_selection(
                &model,
                attempt.claim.effort.as_deref(),
            )
            .map_err(|_| FeatureError::Invalid)?;
            if let Some(run) = self
                .conversations
                .service
                .find_turn(actor, conversation, operation_id, selection, input.clone())
                .await
                .map_err(conversation_error)?
            {
                return Ok(run);
            }
        }
        let (role, revision) = match state.feature.stage {
            FeatureStage::Scope => (TaskRole::Coordinator, state.scope.clone()),
            FeatureStage::Architecture => (TaskRole::Architecture, state.scope.clone()),
            _ => return Err(FeatureError::Conflict),
        };
        if state
            .attempts
            .iter()
            .any(|attempt| attempt.status.holds_reservation())
        {
            return Err(FeatureError::ExecutionUncertain);
        }
        let history = self
            .conversations
            .service
            .history(actor, conversation)
            .await
            .map_err(conversation_error)?;
        let prompt = discussion_prompt(self.codec.as_ref(), &state, role, &history, &input)?;
        let branch = self
            .workspaces
            .initialize(&env, &project, id)
            .await
            .map_err(workspace_error)?;
        let task = internal_task(
            operation_id,
            role,
            "Feature discussion",
            "Discuss the current proposal with the member.",
        );
        let reserved = self
            .reserve_task(
                actor,
                &state,
                TaskRequest {
                    operation_id,
                    attempt_id: operation_id,
                    task,
                    revision,
                    snapshot_id: None,
                    base_oid: branch.base_oid,
                    retry_of: None,
                    input: prompt,
                },
            )
            .await?;
        let scoped = ScopedDiscussion {
            owner: self.clone(),
            reserved,
            conversation,
            proposal: None,
            streams: HashMap::new(),
        };
        self.conversations
            .begin_scoped(scoped, input)
            .await
            .map_err(conversation_error)
    }

    pub(super) async fn resume_discussion(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        state: &FeatureState,
        operation_id: Uuid,
        prior: TaskAttempt,
    ) -> Result<FeatureState, FeatureError> {
        let id = state.feature.id;
        let conversation = state.feature.conversation_id;
        let mut history = self
            .conversations
            .service
            .history(actor, conversation)
            .await
            .map_err(conversation_error)?;
        let failed_run = history
            .runs
            .iter()
            .find(|run| run.operation_id == prior.id())
            .ok_or(FeatureError::Conflict)?
            .id;
        let input = history
            .messages
            .iter()
            .find(|message| message.run_id == failed_run && message.role == MessageRole::User)
            .ok_or(FeatureError::Conflict)?
            .content
            .clone();
        history
            .messages
            .retain(|message| message.run_id != failed_run);
        let prompt = discussion_prompt(
            self.codec.as_ref(),
            state,
            prior.claim.task.role,
            &history,
            &input,
        )?;
        let reserved = self
            .reserve_task(
                actor,
                state,
                TaskRequest {
                    operation_id,
                    attempt_id: operation_id,
                    task: prior.claim.task.clone(),
                    revision: prior.claim.revision.clone(),
                    snapshot_id: None,
                    base_oid: prior.claim.workspace.base_oid.clone(),
                    retry_of: Some(prior.id()),
                    input: prompt,
                },
            )
            .await?;
        let scoped = ScopedDiscussion {
            owner: self.clone(),
            reserved,
            conversation,
            proposal: None,
            streams: HashMap::new(),
        };
        self.conversations
            .begin_scoped(scoped, input)
            .await
            .map_err(conversation_error)?;
        self.store.get(actor, id).await
    }

    pub(super) async fn schedule_architecture(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        env: &Environment,
        project: &Project,
        state: &FeatureState,
    ) -> Result<(), FeatureError> {
        if state
            .attempts
            .iter()
            .any(|attempt| attempt.status.holds_reservation())
        {
            return Ok(());
        }
        let role = if state.current_architecture().is_some() {
            TaskRole::ArchitectureReview
        } else {
            TaskRole::Architecture
        };
        let (task, revision, input) = architecture_request(self.codec.as_ref(), state, role)?;
        if state.attempts.iter().any(|attempt| {
            attempt.claim.task.id == task.id
                && attempt.claim.task.role == role
                && attempt.claim.revision.as_ref() == Some(&revision)
                && attempt.claim.snapshot_id.is_none()
        }) {
            return Ok(());
        }
        let branch = self
            .workspaces
            .initialize(env, project, state.feature.id)
            .await
            .map_err(workspace_error)?;
        let attempt_id = Uuid::new_v4();
        let reserved = self
            .reserve_task(
                actor,
                state,
                TaskRequest {
                    operation_id: attempt_id,
                    attempt_id,
                    task,
                    revision: Some(revision),
                    snapshot_id: None,
                    base_oid: branch.base_oid,
                    retry_of: None,
                    input,
                },
            )
            .await?;
        self.spawn_reserved(reserved);
        Ok(())
    }

    pub(super) async fn accept_automatic_architecture(
        &self,
        actor: EnvironmentActor,
        feature_id: Uuid,
        result: &TaskResult,
    ) -> Result<(), FeatureError> {
        let (env, _, state) = self.context(actor, feature_id).await?;
        let raw = result
            .messages
            .iter()
            .rev()
            .find_map(|message| proposal_part(message).1)
            .ok_or(FeatureError::Invalid)?;
        let proposal = self.codec.parse_proposal(raw)?;
        let FeatureProposal::Architecture { .. } = proposal else {
            return Err(FeatureError::Invalid);
        };
        if state.architecture.is_some() || result.attempt.claim.revision != state.scope {
            return Err(FeatureError::Conflict);
        }
        self.persist_proposal(
            actor,
            &env,
            feature_id,
            result.attempt.claim.revision.clone(),
            proposal,
        )
        .await
    }

    async fn persist_proposal(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        id: Uuid,
        scope: Option<RevisionKey>,
        proposal: FeatureProposal,
    ) -> Result<(), FeatureError> {
        let content = match proposal {
            FeatureProposal::Scope { brief } => GateContent::Scope { brief },
            FeatureProposal::Architecture {
                summary,
                diagrams,
                tasks,
            } => GateContent::Architecture {
                scope: scope.ok_or(FeatureError::Conflict)?,
                summary,
                diagrams,
                tasks,
            },
        };
        let digest = self.codec.revision_digest(&content)?;
        let current = self.store.get(actor, id).await?;
        let previous = match content {
            GateContent::Scope { .. } => current.scope,
            GateContent::Architecture { .. } => current.architecture,
        };
        if previous.as_ref().is_some_and(|key| key.digest == digest) {
            return Ok(());
        }
        self.store
            .apply(
                actor,
                env,
                id,
                Uuid::new_v4(),
                FeatureCommand::ProposeRevision(GateRevision {
                    key: RevisionKey {
                        id: Uuid::new_v4(),
                        digest,
                    },
                    content,
                    approval: None,
                }),
            )
            .await?;
        Ok(())
    }
}

pub(crate) struct ScopedDiscussion {
    owner: Arc<FeatureCoordinator>,
    reserved: ReservedTask,
    conversation: Uuid,
    proposal: Option<FeatureProposal>,
    streams: HashMap<String, StreamTail>,
}

impl ScopedDiscussion {
    pub(crate) fn actor(&self) -> EnvironmentActor {
        self.reserved.actor
    }
    pub(crate) fn conversation(&self) -> Uuid {
        self.conversation
    }
    pub(crate) fn operation(&self) -> Uuid {
        self.reserved.attempt.id()
    }
    pub(crate) fn provider(&self) -> &str {
        &self.reserved.provider
    }
    pub(crate) fn model(&self) -> &str {
        &self.reserved.attempt.claim.model
    }
    pub(crate) fn effort(&self) -> Option<String> {
        self.reserved.attempt.claim.effort.clone()
    }
    pub(crate) fn can_launch(&self) -> bool {
        self.reserved.fresh
    }
    pub(crate) fn failed_before_launch(&self) -> bool {
        self.reserved.attempt.status == AttemptStatus::Failed
            && self.reserved.attempt.process_receipt.is_none()
    }
    pub(crate) fn same_environment(&self, env: &Environment) -> bool {
        env.id == self.reserved.env.id
            && env.generation == self.reserved.env.generation
            && env.image_id == self.reserved.env.image_id
            && env.policy_version == self.reserved.env.policy_version
    }

    pub(crate) async fn abort_preflight(&self) {
        if self.can_launch() {
            let _ = self
                .owner
                .fail_before_launch(&self.reserved, "The conversation could not start")
                .await;
        }
    }

    pub(crate) async fn open(&self) -> Result<ExecutionSession, ExecutionError> {
        self.owner.open_attempt(&self.reserved).await
    }

    pub(crate) async fn thread(&self, id: String) -> Result<(), ConversationError> {
        self.observe(AttemptUpdate::ProviderThread { id }).await
    }

    pub(crate) async fn model_observed(&self, model: String) -> Result<(), ConversationError> {
        self.observe(AttemptUpdate::ObservedModel { model }).await
    }

    async fn observe(&self, update: AttemptUpdate) -> Result<(), ConversationError> {
        self.owner
            .record(&self.reserved, update)
            .await
            .map(|_| ())
            .map_err(|_| ConversationError::Unavailable)
    }

    pub(crate) fn delta(&mut self, provider_item: String, delta: String) -> Option<String> {
        if self.streams.len() >= 128 && !self.streams.contains_key(&provider_item) {
            return None;
        }
        self.streams
            .entry(provider_item)
            .or_default()
            .filter(&delta)
    }

    pub(crate) fn message(&mut self, provider_item: &str, text: String) -> Option<String> {
        self.streams.remove(provider_item);
        let (visible, proposal) = proposal_part(&text);
        if let Some(proposal) = proposal.and_then(|json| self.owner.codec.parse_proposal(json).ok())
        {
            self.proposal = Some(proposal);
        }
        (!visible.trim().is_empty()).then(|| visible.to_owned())
    }

    pub(crate) async fn finish(&mut self, status: RunStatus) -> Result<(), ConversationError> {
        let update = match status {
            RunStatus::Completed => AttemptUpdate::Validated { checkpoint: None },
            RunStatus::Failed => AttemptUpdate::Failed {
                reason: "The provider task failed".into(),
            },
            RunStatus::Interrupted => AttemptUpdate::Interrupted,
            RunStatus::Uncertain => AttemptUpdate::Uncertain,
            RunStatus::Active => return Err(ConversationError::Invalid),
        };
        self.observe(update).await?;
        if status == RunStatus::Completed
            && let Some(proposal) = self.proposal.take()
        {
            let id = self.reserved.attempt.claim.workspace.feature_id;
            let revision = self.reserved.attempt.claim.revision.clone();
            if self
                .owner
                .persist_proposal(
                    self.reserved.actor,
                    &self.reserved.env,
                    id,
                    revision,
                    proposal,
                )
                .await
                .is_ok()
            {
                self.owner.schedule(self.reserved.actor, id);
            }
        }
        Ok(())
    }
}

fn internal_task(id: Uuid, role: TaskRole, title: &str, instructions: &str) -> FeatureTask {
    FeatureTask {
        id,
        title: title.into(),
        instructions: instructions.into(),
        role,
        paths: Vec::new(),
        checks: Vec::new(),
        depends_on: Vec::new(),
    }
}

pub(super) fn architecture_request(
    codec: &dyn FeatureCodec,
    state: &FeatureState,
    role: TaskRole,
) -> Result<(FeatureTask, RevisionKey, String), FeatureError> {
    let scope = state.current_scope().ok_or(FeatureError::Conflict)?;
    let GateContent::Scope { brief } = &scope.content else {
        return Err(FeatureError::Conflict);
    };
    let instructions = bundle::role_instructions(role);
    match role {
        TaskRole::Architecture => Ok((
            internal_task(
                scope.key.id,
                role,
                "Prepare architecture",
                "Turn the approved brief into a reviewable design.",
            ),
            scope.key.clone(),
            format!(
                "{instructions}\n\nApproved brief:\n{brief}\n\n{}\nPrepare an architecture proposal now.",
                architecture_protocol()
            ),
        )),
        TaskRole::ArchitectureReview => {
            let revision = state.current_architecture().ok_or(FeatureError::Conflict)?;
            let design = codec.render_revision(&revision.content)?;
            Ok((
                internal_task(
                    revision.key.id,
                    role,
                    "Critique architecture",
                    "Review the proposed architecture and its implementation risks.",
                ),
                revision.key.clone(),
                format!(
                    "{instructions}\n\nApproved brief:\n{brief}\n\nCurrent proposed architecture:\n{design}\n\nReturn actionable design feedback, or say the design is ready for the member's review."
                ),
            ))
        }
        _ => Err(FeatureError::Conflict),
    }
}

fn discussion_prompt(
    codec: &dyn FeatureCodec,
    state: &FeatureState,
    role: TaskRole,
    history: &ConversationHistory,
    input: &str,
) -> Result<String, FeatureError> {
    if history.messages_truncated {
        return Err(FeatureError::Invalid);
    }
    let scope = state.current_scope().map(|revision| &revision.content);
    let architecture = state
        .current_architecture()
        .map(|revision| &revision.content);
    let mut plan = String::new();
    for proposal in [scope, architecture].into_iter().flatten() {
        plan.push_str(&codec.render_revision(proposal)?);
        plan.push('\n');
    }
    let mut prompt = format!(
        "{}\n\n{}\n\nCurrent proposals:\n{plan}\n\nVisible conversation:\n",
        bundle::role_instructions(role),
        if role == TaskRole::Architecture {
            architecture_protocol()
        } else {
            scope_protocol()
        },
    );
    for message in &history.messages {
        let role = match message.role {
            MessageRole::User => "User",
            MessageRole::Assistant => "Assistant",
        };
        prompt.push_str(role);
        prompt.push_str(":\n");
        prompt.push_str(&message.content);
        prompt.push_str("\n\n");
        if prompt.len() > PROMPT_LIMIT {
            return Err(FeatureError::Invalid);
        }
    }
    prompt.push_str("Current user message:\n");
    prompt.push_str(input);
    if prompt.len() > PROMPT_LIMIT {
        return Err(FeatureError::Invalid);
    }
    Ok(prompt)
}

fn scope_protocol() -> &'static str {
    r#"Talk naturally in the user's language. Ask about unresolved product decisions. When the brief is ready or revised, append one fenced factory-proposal JSON block to the very end of your last message, with exactly {"kind":"scope","brief":"reviewable Markdown brief"}. The user approves the revision separately. Do not claim approval or include this block in prose."#
}

fn architecture_protocol() -> &'static str {
    r#"Talk naturally in the user's language. When the design is ready or revised, append one fenced factory-proposal JSON block to the very end of your last message. Use this schema exactly:
{"kind":"architecture","summary":"implementation decisions","diagrams":[{"title":"Name","source":"Mermaid source"}],"tasks":[{"id":"unique UUID","title":"Task title","instructions":"Specific expected change","role":"Implementation","paths":["owned/file"],"checks":["reproducible command"],"depends_on":[]}]}
Use UUID strings and existing task UUID strings for prerequisites. Available task roles are Implementation, ComplexImplementation, Visual and Triage. Only implementation roles can own write paths. Assign explicit existing repository test commands and use a trailing slash for a directory scope. At least one task needs a check. If the member is changing an approved product decision, use {"kind":"scope","brief":"updated product brief"} instead and explain the decision. The user approves separately. Do not execute the task graph."#
}

pub(super) fn valid_architecture_reply(codec: &dyn FeatureCodec, messages: &[String]) -> bool {
    messages
        .iter()
        .rev()
        .find_map(|message| proposal_part(message).1)
        .and_then(|value| codec.parse_proposal(value).ok())
        .is_some_and(|proposal| matches!(proposal, FeatureProposal::Architecture { .. }))
}

pub(super) fn proposal_part(text: &str) -> (&str, Option<&str>) {
    let Some((visible, hidden)) = text.split_once(MARKER) else {
        return (text, None);
    };
    let proposal = hidden
        .strip_prefix('\n')
        .and_then(|body| body.trim_end().strip_suffix("```"))
        .map(str::trim);
    (visible.trim_end(), proposal)
}

#[derive(Default)]
struct StreamTail {
    tail: String,
    hidden: bool,
}

impl StreamTail {
    fn filter(&mut self, delta: &str) -> Option<String> {
        if self.hidden {
            return None;
        }
        self.tail.push_str(delta);
        if let Some(index) = self.tail.find(MARKER) {
            self.hidden = true;
            let visible = self.tail[..index].to_owned();
            self.tail.clear();
            return (!visible.is_empty()).then_some(visible);
        }
        let keep = (1..MARKER.len())
            .rev()
            .find(|&n| self.tail.ends_with(&MARKER[..n]))
            .unwrap_or(0);
        let remaining = self.tail.split_off(self.tail.len() - keep);
        let visible = std::mem::replace(&mut self.tail, remaining);
        (!visible.is_empty()).then_some(visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_marker_does_not_leak_the_internal_task_graph() {
        let mut stream = StreamTail::default();
        assert_eq!(stream.filter("Ready\n``"), Some("Ready\n".into()));
        assert_eq!(
            stream.filter("`factory-proposal\n{\"kind\":\"scope\"}"),
            None
        );
        assert_eq!(stream.filter("\n```"), None);
        let (visible, json) = proposal_part(
            "Ready\n```factory-proposal\n{\"kind\":\"scope\",\"brief\":\"Save a file\"}\n```",
        );
        assert_eq!(visible, "Ready");
        assert_eq!(json, Some(r#"{"kind":"scope","brief":"Save a file"}"#));
    }
}
