use super::{FeatureCoordinator, bundle, conversation_error, profiles, workspace_error};
use factory_core::{
    conversations::execution::{ExecutionError, ExecutionEvent, ExecutionSession},
    environments::{Environment, EnvironmentActor},
    features::*,
    projects::Project,
};
use std::sync::Arc;
use uuid::Uuid;

pub(super) struct TaskRequest {
    pub operation_id: Uuid,
    pub attempt_id: Uuid,
    pub task: FeatureTask,
    pub revision: Option<RevisionKey>,
    pub snapshot_id: Option<Uuid>,
    pub base_oid: String,
    pub retry_of: Option<Uuid>,
    pub input: String,
}

pub(super) struct TaskResult {
    pub attempt: TaskAttempt,
    pub messages: Vec<String>,
}

pub(super) struct ReservedTask {
    pub actor: EnvironmentActor,
    pub env: Environment,
    pub project: Project,
    pub provider: String,
    pub attempt: TaskAttempt,
    pub input: String,
    pub fresh: bool,
}

impl FeatureCoordinator {
    pub(super) async fn reserve_task(
        &self,
        actor: EnvironmentActor,
        feature: &FeatureState,
        request: TaskRequest,
    ) -> Result<ReservedTask, FeatureError> {
        let id = feature.feature.id;
        let (env, project, state) = self.context(actor, id).await?;
        let provider = state.feature.provider;
        if state.feature.workflow_revision != bundle::WORKFLOW_REVISION {
            return Err(FeatureError::Unavailable);
        }
        let prior = request
            .retry_of
            .and_then(|id| state.attempts.iter().find(|attempt| attempt.id() == id));
        let replay = state
            .attempts
            .iter()
            .find(|attempt| attempt.id() == request.attempt_id);
        let catalog = if replay.is_some() {
            Ok(Vec::new())
        } else {
            self.conversations.models(actor).await
        };
        if let Err(
            error @ (factory_core::conversations::ConversationError::Unauthorized
            | factory_core::conversations::ConversationError::NotFound),
        ) = catalog
        {
            return Err(conversation_error(error));
        }
        let target =
            profiles::preferred(&provider, request.task.role).ok_or(FeatureError::Invalid)?;
        let assigned = catalog.ok().and_then(|models| {
            if let Some(prior) = prior.filter(|p| p.process_receipt.is_some()) {
                return models
                    .iter()
                    .find(|candidate| {
                        let prefix = format!("factory:{provider}:");
                        candidate.id.strip_prefix(&prefix).is_some_and(|native| {
                            native == prior.claim.model
                                || candidate.resolved_model.as_deref() == Some(&prior.claim.model)
                        }) && prior
                            .claim
                            .effort
                            .as_ref()
                            .is_none_or(|effort| candidate.effort_levels.contains(effort))
                    })
                    .map(|_| profiles::ModelSelection {
                        model: prior.claim.model.clone(),
                        effort: prior.claim.effort.clone(),
                    });
            }
            profiles::resolve(&provider, request.task.role, &models).ok()
        });
        let (model, effort) = if let Some(replay) = replay {
            (replay.claim.model.clone(), replay.claim.effort.clone())
        } else if let Some(selection) = &assigned {
            (selection.model.clone(), selection.effort.clone())
        } else {
            (
                prior
                    .filter(|p| p.process_receipt.is_some())
                    .map_or_else(|| target.to_owned(), |p| p.claim.model.clone()),
                prior.and_then(|p| p.claim.effort.clone()),
            )
        };
        let input = if request.snapshot_id.is_some() && request.task.role.is_reviewer() {
            format!(
                "{}\n\n{}",
                bundle::role_instructions(request.task.role),
                request.input
            )
        } else {
            request.input
        };
        let workspace = TaskWorkspace {
            feature_id: id,
            attempt_id: request.attempt_id,
            base_oid: request.base_oid,
            access: request.task.role.access(),
        };
        let _gate = self
            .conversations
            .lock_feature_account(actor)
            .await
            .map_err(conversation_error)?;
        let connected = self
            .conversations
            .require_connected(actor, &provider)
            .await
            .is_ok();
        let receipt = self
            .store
            .apply(
                actor,
                &env,
                id,
                request.operation_id,
                FeatureCommand::ClaimAttempt(AttemptClaim {
                    task: request.task,
                    revision: request.revision,
                    workspace,
                    snapshot_id: request.snapshot_id,
                    model,
                    effort,
                    skill_revision: bundle::WORKFLOW_REVISION.into(),
                    retry_of: request.retry_of,
                }),
            )
            .await?;
        let mut reserved = ReservedTask {
            actor,
            env,
            project,
            provider,
            attempt: find_attempt(receipt.state, request.attempt_id)?,
            input,
            fresh: receipt.fresh,
        };
        if reserved.fresh && (assigned.is_none() || !connected) {
            let reason = if connected {
                format!(
                    "Required model is unavailable: {}",
                    reserved.attempt.claim.model
                )
            } else {
                format!("The {} account is not connected", reserved.provider)
            };
            reserved.attempt = self.fail_before_launch(&reserved, &reason).await?;
            reserved.fresh = false;
        }
        Ok(reserved)
    }

    pub(super) async fn record(
        &self,
        task: &ReservedTask,
        update: AttemptUpdate,
    ) -> Result<TaskAttempt, FeatureError> {
        let receipt = self
            .store
            .apply(
                task.actor,
                &task.env,
                task.attempt.claim.workspace.feature_id,
                Uuid::new_v4(),
                FeatureCommand::RecordAttempt {
                    attempt_id: task.attempt.id(),
                    update,
                },
            )
            .await?;
        find_attempt(receipt.state, task.attempt.id())
    }

    pub(super) async fn fail_before_launch(
        &self,
        task: &ReservedTask,
        reason: &str,
    ) -> Result<TaskAttempt, FeatureError> {
        self.record(
            task,
            AttemptUpdate::LaunchSettled {
                process_receipt: None,
            },
        )
        .await?;
        self.record(
            task,
            AttemptUpdate::Failed {
                reason: reason.into(),
            },
        )
        .await
    }

    pub(super) async fn open_attempt(
        &self,
        task: &ReservedTask,
    ) -> Result<ExecutionSession, ExecutionError> {
        let workspace = &task.attempt.claim.workspace;
        if let Err(error) = self
            .workspaces
            .prepare_task(&task.env, &task.project, workspace)
            .await
        {
            self.fail_before_launch(task, &error.to_string())
                .await
                .map_err(|_| ExecutionError::Unavailable)?;
            return Err(ExecutionError::PreflightUnavailable);
        }
        let current = self
            .conversations
            .service
            .current_environment(task.actor)
            .await;
        if !current.as_ref().is_ok_and(|env| {
            env.id == task.env.id
                && env.generation == task.env.generation
                && env.image_id == task.env.image_id
                && env.policy_version == task.env.policy_version
        }) {
            self.fail_before_launch(task, "The environment changed before launch")
                .await
                .map_err(|_| ExecutionError::Unavailable)?;
            return Err(ExecutionError::PreflightUnavailable);
        }
        let request = FeatureRun {
            workspace: workspace.clone(),
            provider: task.provider.clone(),
            model: task.attempt.claim.model.clone(),
            effort: task.attempt.claim.effort.clone(),
            input: task.input.clone(),
        };
        match self
            .runtime
            .open_task(&task.env, &task.project, &request)
            .await
        {
            Ok(mut opened) => {
                if self
                    .record(
                        task,
                        AttemptUpdate::LaunchSettled {
                            process_receipt: Some(opened.process_receipt),
                        },
                    )
                    .await
                    .is_err()
                {
                    let _ = opened.session.control.interrupt().await;
                    return Err(ExecutionError::CleanupUncertain);
                }
                Ok(opened.session)
            }
            Err(error) => {
                let result = match error {
                    ExecutionError::PreflightUnavailable | ExecutionError::Invalid => {
                        self.fail_before_launch(task, &error.to_string()).await
                    }
                    ExecutionError::CleanupUncertain => {
                        self.record(
                            task,
                            AttemptUpdate::LaunchSettled {
                                process_receipt: None,
                            },
                        )
                        .await
                        .map_err(|_| ExecutionError::CleanupUncertain)?;
                        self.record(task, AttemptUpdate::Uncertain).await
                    }
                    _ => self.record(task, AttemptUpdate::Uncertain).await,
                };
                result.map_err(|_| ExecutionError::CleanupUncertain)?;
                Err(error)
            }
        }
    }

    pub(super) async fn run_task(
        &self,
        actor: EnvironmentActor,
        feature: &FeatureState,
        request: TaskRequest,
    ) -> Result<TaskResult, FeatureError> {
        let task = self.reserve_task(actor, feature, request).await?;
        self.run_reserved(task).await
    }

    pub(super) async fn run_reserved(
        &self,
        task: ReservedTask,
    ) -> Result<TaskResult, FeatureError> {
        if !task.fresh {
            return Ok(saved_result(task.attempt));
        }
        let mut session = match self.open_attempt(&task).await {
            Ok(session) => session,
            Err(_) => {
                let state = self
                    .store
                    .get(task.actor, task.attempt.claim.workspace.feature_id)
                    .await?;
                return Ok(saved_result(find_attempt(state, task.attempt.id())?));
            }
        };
        let mut ticker = self.conversations.ticker();
        let mut messages = Vec::new();
        let mut retained = 0;
        let mut started = false;
        let terminal = loop {
            let event = match ticker.next(session.output.as_mut()).await {
                Some(Ok(Some(event))) => event,
                None => {
                    let current = self
                        .conversations
                        .service
                        .current_environment(task.actor)
                        .await;
                    if current.as_ref().is_ok_and(|env| {
                        env.id == task.env.id && env.generation == task.env.generation
                    }) {
                        continue;
                    }
                    break AttemptUpdate::Uncertain;
                }
                _ => break AttemptUpdate::Uncertain,
            };
            let result = match event {
                ExecutionEvent::ThreadReady { provider_thread } if !started => {
                    started = true;
                    match self
                        .record(
                            &task,
                            AttemptUpdate::ProviderThread {
                                id: provider_thread,
                            },
                        )
                        .await
                    {
                        Ok(_) => session
                            .control
                            .begin_turn()
                            .await
                            .map_err(|_| FeatureError::ExecutionUncertain),
                        Err(error) => Err(error),
                    }
                }
                ExecutionEvent::ModelObserved { model } => self
                    .record(&task, AttemptUpdate::ObservedModel { model })
                    .await
                    .map(|_| ()),
                ExecutionEvent::Message { text, .. } if retained + text.len() <= 65_536 => {
                    retained += text.len();
                    let visible = if task.attempt.claim.task.role == TaskRole::Architecture {
                        super::discussion::proposal_part(&text).0
                    } else {
                        &text
                    };
                    let summary = bounded(visible, 4096);
                    messages.push(text);
                    if summary.trim().is_empty() {
                        Ok(())
                    } else {
                        self.record(&task, AttemptUpdate::Activity { summary })
                            .await
                            .map(|_| ())
                    }
                }
                ExecutionEvent::Message { .. } => break AttemptUpdate::Uncertain,
                ExecutionEvent::Completed if started => {
                    break self.validate_completed(&task, &messages).await;
                }
                ExecutionEvent::Failed => {
                    break AttemptUpdate::Failed {
                        reason: "The provider task failed".into(),
                    };
                }
                ExecutionEvent::Interrupted => break AttemptUpdate::Interrupted,
                ExecutionEvent::Delta { .. } | ExecutionEvent::ResponseDelivered { .. } => Ok(()),
                _ => break AttemptUpdate::Uncertain,
            };
            if result.is_err() {
                break AttemptUpdate::Uncertain;
            }
        };
        if terminal == AttemptUpdate::Uncertain {
            let _ = session.control.interrupt().await;
        }
        let attempt = self.record(&task, terminal).await?;
        Ok(TaskResult { attempt, messages })
    }

    async fn validate_completed(&self, task: &ReservedTask, messages: &[String]) -> AttemptUpdate {
        let role = task.attempt.claim.task.role;
        let result = if let Some(snapshot_id) = task
            .attempt
            .claim
            .snapshot_id
            .filter(|_| role.is_reviewer())
        {
            let state = match self
                .store
                .get(task.actor, task.attempt.claim.workspace.feature_id)
                .await
            {
                Ok(state) => state,
                Err(_) => return AttemptUpdate::Uncertain,
            };
            let Some(round) = state
                .review()
                .filter(|round| round.snapshot.id == snapshot_id)
            else {
                return AttemptUpdate::Failed {
                    reason: "The reviewed snapshot changed".into(),
                };
            };
            let Some(reply) = messages.iter().rev().find(|text| {
                text.trim().len() <= 32_768
                    && super::review::parse_reply(
                        self.codec.as_ref(),
                        text,
                        &round.snapshot.git.patch_digest,
                    )
                    .is_ok()
            }) else {
                return AttemptUpdate::Failed {
                    reason: "The reviewer did not return a valid reply for this snapshot".into(),
                };
            };
            reply.trim().to_owned()
        } else {
            if role == TaskRole::Architecture
                && !super::discussion::valid_architecture_reply(self.codec.as_ref(), messages)
            {
                return AttemptUpdate::Failed {
                    reason: "The architect did not return a structured proposal".into(),
                };
            }
            bounded(&messages.join("\n\n"), 32_768)
        };
        if result.trim().is_empty() {
            if role.is_reviewer() {
                return AttemptUpdate::Failed {
                    reason: "The reviewer did not return feedback".into(),
                };
            }
        } else if self
            .record(task, AttemptUpdate::Result { text: result })
            .await
            .is_err()
        {
            return AttemptUpdate::Uncertain;
        }
        if task.attempt.claim.workspace.access == WorkspaceAccess::ReadOnly {
            return AttemptUpdate::Validated { checkpoint: None };
        }
        match self.validate_writer(task).await {
            Ok(checkpoint) => AttemptUpdate::Validated {
                checkpoint: Some(checkpoint),
            },
            Err(outcome) => outcome,
        }
    }

    async fn validate_writer(&self, task: &ReservedTask) -> Result<String, AttemptUpdate> {
        let scope = &task.attempt.claim.workspace;
        for command in &task.attempt.claim.task.checks {
            match self
                .runtime
                .run_check(&task.env, &task.project, scope, command)
                .await
            {
                Ok(result) if result.exit_code == 0 => {}
                Ok(result) => {
                    return Err(AttemptUpdate::Failed {
                        reason: bounded(
                            &format!(
                                "Check exited with {}: {command}\n{}",
                                result.exit_code, result.output
                            ),
                            4096,
                        ),
                    });
                }
                Err(ExecutionError::LaunchUncertain | ExecutionError::CleanupUncertain) => {
                    return Err(AttemptUpdate::Uncertain);
                }
                Err(error) => {
                    return Err(AttemptUpdate::Failed {
                        reason: error.to_string(),
                    });
                }
            }
        }
        let failure = |error: WorkspaceError| {
            if error == WorkspaceError::Uncertain {
                AttemptUpdate::Uncertain
            } else {
                AttemptUpdate::Failed {
                    reason: error.to_string(),
                }
            }
        };
        let checkpoint = self
            .workspaces
            .checkpoint(&task.env, &task.project, &task.attempt)
            .await
            .map_err(failure)?;
        // The controller compares the expected head. A moved head is a normal
        // race between independent writers, so inspect it again before retrying.
        for _ in 0..64 {
            let current = self
                .workspaces
                .snapshot(&task.env, &task.project, scope.feature_id)
                .await
                .map_err(failure)?;
            match self
                .workspaces
                .integrate(
                    &task.env,
                    &task.project,
                    &task.attempt,
                    &current.git.head_oid,
                )
                .await
            {
                Ok(_) => return Ok(checkpoint),
                Err(WorkspaceError::Conflict) => {
                    let latest = self
                        .workspaces
                        .snapshot(&task.env, &task.project, scope.feature_id)
                        .await
                        .map_err(failure)?;
                    if latest.git.head_oid == current.git.head_oid {
                        return if latest.git.head_oid == scope.base_oid {
                            Err(failure(WorkspaceError::Conflict))
                        } else {
                            Err(AttemptUpdate::IntegrationConflict {
                                retry_base: latest.git.head_oid,
                            })
                        };
                    }
                }
                Err(error) => return Err(failure(error)),
            }
        }
        Err(failure(WorkspaceError::Conflict))
    }

    pub(super) fn schedule(self: &Arc<Self>, actor: EnvironmentActor, feature: Uuid) {
        let owner = self.clone();
        self.conversations.spawn(Box::pin(async move {
            let Ok(gate) = owner.driver(feature) else {
                return;
            };
            let _drive = gate.lock().await;
            let _ = owner.drive(actor, feature).await;
        }));
    }

    async fn drive(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        feature: Uuid,
    ) -> Result<(), FeatureError> {
        let (env, project, state) = self.context(actor, feature).await?;
        match state.feature.stage {
            FeatureStage::Architecture => {
                return self
                    .schedule_architecture(actor, &env, &project, &state)
                    .await;
            }
            FeatureStage::Execution => {}
            FeatureStage::Review => return self.advance_review(actor, feature).await,
            _ => return Ok(()),
        }
        let architecture = state.current_architecture().ok_or(FeatureError::Conflict)?;
        let GateContent::Architecture { tasks, .. } = &architecture.content else {
            return Err(FeatureError::Conflict);
        };
        let completed = |id| {
            latest_code_attempt(&state, &architecture.key, id)
                .is_some_and(|attempt| attempt.status == AttemptStatus::Validated)
        };
        if tasks.iter().all(|task| completed(task.id)) {
            return self.advance_review(actor, feature).await;
        }
        let available: Vec<_> = tasks
            .iter()
            .filter(|task| {
                latest_code_attempt(&state, &architecture.key, task.id).is_none()
                    && task.depends_on.iter().all(|id| completed(*id))
            })
            .cloned()
            .collect();
        if available.is_empty() {
            return Ok(());
        }
        let head = self
            .workspaces
            .snapshot(&env, &project, feature)
            .await
            .map_err(workspace_error)?
            .git
            .head_oid;
        for task in available {
            let attempt_id = Uuid::new_v4();
            let input = task_prompt(&state, &task)?;
            let request = TaskRequest {
                operation_id: attempt_id,
                attempt_id,
                revision: Some(architecture.key.clone()),
                snapshot_id: None,
                base_oid: head.clone(),
                retry_of: None,
                task,
                input,
            };
            let reserved = match self.reserve_task(actor, &state, request).await {
                Ok(reserved) => reserved,
                Err(FeatureError::Conflict | FeatureError::ExecutionUncertain) => continue,
                Err(error) => return Err(error),
            };
            self.spawn_reserved(reserved);
        }
        Ok(())
    }

    pub(super) fn spawn_reserved(self: &Arc<Self>, reserved: ReservedTask) {
        if !reserved.fresh {
            return;
        }
        let actor = reserved.actor;
        let feature = reserved.attempt.claim.workspace.feature_id;
        let role = reserved.attempt.claim.task.role;
        let owner = self.clone();
        self.conversations.spawn(Box::pin(async move {
            if let Ok(result) = owner.run_reserved(reserved).await {
                if result.attempt.status == AttemptStatus::Validated
                    && role == TaskRole::Architecture
                {
                    let _ = owner
                        .accept_automatic_architecture(actor, feature, &result)
                        .await;
                }
                owner.schedule(actor, feature);
            }
        }));
    }
}

pub(super) fn task_prompt(
    state: &FeatureState,
    task: &FeatureTask,
) -> Result<String, FeatureError> {
    let scope = state.current_scope().ok_or(FeatureError::Conflict)?;
    let architecture = state.current_architecture().ok_or(FeatureError::Conflict)?;
    let GateContent::Scope { brief } = &scope.content else {
        return Err(FeatureError::Conflict);
    };
    let GateContent::Architecture {
        summary, diagrams, ..
    } = &architecture.content
    else {
        return Err(FeatureError::Conflict);
    };
    let mut diagram_text = String::new();
    for diagram in diagrams {
        use std::fmt::Write;
        let _ = writeln!(diagram_text, "\n{}:\n{}", diagram.title, diagram.source);
    }
    let paths = task.paths.join(", ");
    let checks = task.checks.join("\n");
    let role = bundle::role_instructions(task.role);
    Ok(format!(
        "{role}\n\nApproved product brief:\n{brief}\n\nApproved design:\n{summary}\n{diagram_text}\n\nTask: {}\n{}\nOwned paths: {paths}\nApproved checks:\n{checks}",
        task.title, task.instructions
    ))
}

fn latest_code_attempt<'a>(
    state: &'a FeatureState,
    revision: &RevisionKey,
    task: Uuid,
) -> Option<&'a TaskAttempt> {
    state.attempts.iter().rev().find(|attempt| {
        attempt.claim.revision.as_ref() == Some(revision)
            && attempt.claim.task.id == task
            && attempt.claim.snapshot_id.is_none()
    })
}

fn find_attempt(state: FeatureState, id: Uuid) -> Result<TaskAttempt, FeatureError> {
    state
        .attempts
        .into_iter()
        .find(|attempt| attempt.id() == id)
        .ok_or(FeatureError::Conflict)
}

fn saved_result(attempt: TaskAttempt) -> TaskResult {
    let messages = attempt.result.iter().cloned().collect();
    TaskResult { attempt, messages }
}

pub(super) fn bounded(text: &str, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].replace('\0', "")
}
