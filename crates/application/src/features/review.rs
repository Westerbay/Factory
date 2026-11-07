use super::{
    FeatureCoordinator,
    coordinator::{TaskRequest, approved_context},
};
use factory_core::{
    environments::{Environment, EnvironmentActor},
    features::{
        AttemptStatus, FeatureCodec, FeatureCommand, FeatureError, FeatureReplyFinding,
        FeatureStage, FeatureState, FeatureTask, GateContent, GateRevision, ReviewCheck,
        ReviewFinding, ReviewRound, ReviewSnapshot, ReviewVerdict, RevisionKey, TaskRole,
        TaskWorkspace, WorkspaceAccess, WorkspaceDiff, WorkspaceError,
    },
    projects::Project,
};
use std::{collections::BTreeSet, sync::Arc};
use tokio::sync::mpsc;
use uuid::Uuid;

pub(super) fn parse_reply(
    codec: &dyn FeatureCodec,
    text: &str,
    expected_digest: &str,
) -> Result<Vec<FeatureReplyFinding>, FeatureError> {
    let text = text.trim();
    let source = text
        .strip_prefix("```json")
        .and_then(|body| body.strip_suffix("```"))
        .unwrap_or(text)
        .trim();
    if source.len() > 32_768 {
        return Err(FeatureError::Invalid);
    }
    let reply = codec.parse_review(source)?;
    if reply.patch_digest != expected_digest {
        return Err(FeatureError::StaleSnapshot);
    }
    if !reply.complete {
        return Err(FeatureError::ReviewBlocked);
    }
    if reply.findings.len() > 32
        || reply.findings.iter().any(|finding| {
            finding.summary.trim().is_empty()
                || finding.summary.len() > 4000
                || finding.summary.contains('\0')
        })
    {
        return Err(FeatureError::Invalid);
    }
    Ok(reply.findings)
}

impl FeatureCoordinator {
    /// Drive one stable integrated snapshot at a time. Re-entry uses durable
    /// reviewer attempts; only missing work is dispatched automatically.
    pub(super) async fn advance_review(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        feature_id: Uuid,
    ) -> Result<(), FeatureError> {
        loop {
            let (env, project, mut feature) = self.context(actor, feature_id).await?;
            if !matches!(
                feature.feature.stage,
                FeatureStage::Execution | FeatureStage::Review
            ) {
                return Ok(());
            }
            let diff = self.feature_diff(&env, &project, feature_id).await?;
            let reviewed_diff_changed = feature.review().is_some_and(|round| {
                round.verdict.as_ref().is_some_and(ReviewVerdict::ready)
                    && round.snapshot.git != diff.git
            });
            if feature.feature.stage == FeatureStage::Execution || reviewed_diff_changed {
                let snapshot = ReviewSnapshot {
                    id: Uuid::new_v4(),
                    git: diff.git.clone(),
                };
                feature = self
                    .store
                    .apply(
                        actor,
                        &env,
                        feature_id,
                        Uuid::new_v4(),
                        FeatureCommand::BeginReview {
                            snapshot,
                            required_roles: review_roles(&diff),
                        },
                    )
                    .await?
                    .state;
            }
            let round = feature.review().ok_or(FeatureError::Conflict)?.clone();
            if round.snapshot.git != diff.git {
                return Err(FeatureError::StaleSnapshot);
            }
            if round.verdict.is_none() {
                self.launch_reviewers(actor, &feature, &round, &diff)
                    .await?;
                feature = self.store.get(actor, feature_id).await?;
                let mut findings = collect_findings(self.codec.as_ref(), &feature, &round)?;
                if let Some(finding) = scope_finding(&feature, &diff)? {
                    findings.push(finding);
                }
                let checks = self
                    .verify_integrated(&env, &project, &feature, &round)
                    .await?;
                if self.feature_diff(&env, &project, feature_id).await?.git != round.snapshot.git {
                    return Err(FeatureError::StaleSnapshot);
                }
                let verdict = ReviewVerdict { findings, checks };
                feature = self
                    .store
                    .apply(
                        actor,
                        &env,
                        feature_id,
                        self.codec.derived_id(round.snapshot.id, "verdict"),
                        FeatureCommand::RecordReview {
                            snapshot_id: round.snapshot.id,
                            patch_digest: round.snapshot.git.patch_digest.clone(),
                            verdict,
                        },
                    )
                    .await?
                    .state;
            }
            let Some(verdict) = feature.review().and_then(|review| review.verdict.as_ref()) else {
                return Err(FeatureError::ReviewBlocked);
            };
            if verdict.ready() {
                return Ok(());
            }
            if scope_finding(&feature, &diff)?.is_some() {
                return Err(FeatureError::ReviewBlocked);
            }
            let used = feature
                .reviews
                .iter()
                .filter(|item| item.architecture == round.architecture)
                .count();
            if used >= 2 + feature.additional_review_rounds as usize {
                return Err(FeatureError::ReviewBlocked);
            }
            let request = correction_request(self.codec.as_ref(), &feature, &round, verdict)?;
            feature = self
                .store
                .apply(
                    actor,
                    &env,
                    feature_id,
                    self.codec
                        .derived_id(round.snapshot.id, "begin-corrections"),
                    FeatureCommand::BeginCorrections {
                        snapshot_id: round.snapshot.id,
                    },
                )
                .await?
                .state;
            let result = self.run_task(actor, &feature, request).await?;
            if result.attempt.status != AttemptStatus::Validated {
                return Err(FeatureError::ReviewBlocked);
            }
        }
    }

    pub async fn reviewed_diff(
        &self,
        actor: EnvironmentActor,
        feature_id: Uuid,
    ) -> Result<WorkspaceDiff, FeatureError> {
        let (env, project, feature) = self.context(actor, feature_id).await?;
        if !matches!(
            feature.feature.stage,
            FeatureStage::Review | FeatureStage::Result | FeatureStage::Draft
        ) {
            return Err(FeatureError::Conflict);
        }
        self.feature_diff(&env, &project, feature_id).await
    }

    pub async fn authorize_additional_review(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        feature_id: Uuid,
        operation_id: Uuid,
        snapshot_id: Uuid,
    ) -> Result<FeatureState, FeatureError> {
        let (env, project, feature) = self.context(actor, feature_id).await?;
        if matches!(
            feature.feature.stage,
            FeatureStage::Result | FeatureStage::Review
        ) && let Some(round) = feature.review().filter(|round| {
            round.snapshot.id == snapshot_id
                && round.verdict.as_ref().is_some_and(ReviewVerdict::ready)
        }) {
            let observed = self.feature_diff(&env, &project, feature_id).await?;
            if observed.git != round.snapshot.git {
                let state = self
                    .store
                    .apply(
                        actor,
                        &env,
                        feature_id,
                        operation_id,
                        FeatureCommand::InvalidateResult { snapshot_id },
                    )
                    .await?
                    .state;
                let used = state
                    .reviews
                    .iter()
                    .filter(|review| review.architecture == round.architecture)
                    .count();
                let state = if used >= 2 + state.additional_review_rounds as usize {
                    self.store
                        .apply(
                            actor,
                            &env,
                            feature_id,
                            self.codec.derived_id(operation_id, "extend-stale-review"),
                            FeatureCommand::AuthorizeAdditionalReview { snapshot_id },
                        )
                        .await?
                        .state
                } else {
                    state
                };
                self.schedule(actor, feature_id);
                return Ok(state);
            }
        }
        let result = self
            .store
            .apply(
                actor,
                &env,
                feature_id,
                operation_id,
                FeatureCommand::AuthorizeAdditionalReview { snapshot_id },
            )
            .await;
        let receipt = match result {
            // After a new snapshot becomes current, recover the original
            // invalidate receipt by its exact operation and snapshot IDs.
            Err(FeatureError::Conflict | FeatureError::StaleSnapshot)
                if feature.current_review != Some(snapshot_id)
                    && feature.reviews.iter().any(|round| {
                        round.snapshot.id == snapshot_id
                            && round.verdict.as_ref().is_some_and(ReviewVerdict::ready)
                    }) =>
            {
                self.store
                    .apply(
                        actor,
                        &env,
                        feature_id,
                        operation_id,
                        FeatureCommand::InvalidateResult { snapshot_id },
                    )
                    .await?
            }
            result => result?,
        };
        self.schedule(actor, feature_id);
        Ok(receipt.state)
    }

    pub async fn reopen_gate(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        feature_id: Uuid,
        operation_id: Uuid,
        revision: RevisionKey,
    ) -> Result<FeatureState, FeatureError> {
        let (env, _, feature) = self.context(actor, feature_id).await?;
        let proposal = feature
            .revisions
            .iter()
            .find(|item| item.key == revision)
            .ok_or(FeatureError::Conflict)?;
        let current = match &proposal.content {
            GateContent::Scope { .. } => &feature.scope,
            GateContent::Architecture { .. } => &feature.architecture,
        };
        if current.as_ref() != Some(&revision)
            && current.as_ref().is_none_or(|key| key.id != operation_id)
        {
            return Err(FeatureError::Conflict);
        }
        let receipt = self
            .store
            .apply(
                actor,
                &env,
                feature_id,
                operation_id,
                FeatureCommand::ProposeRevision(GateRevision {
                    key: RevisionKey {
                        id: operation_id,
                        digest: revision.digest,
                    },
                    content: proposal.content.clone(),
                    approval: None,
                }),
            )
            .await?;
        self.schedule(actor, feature_id);
        Ok(receipt.state)
    }

    async fn feature_diff(
        &self,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
    ) -> Result<WorkspaceDiff, FeatureError> {
        self.workspaces
            .snapshot(env, project, feature_id)
            .await
            .map_err(workspace_error)
    }

    async fn launch_reviewers(
        self: &Arc<Self>,
        actor: EnvironmentActor,
        feature: &FeatureState,
        round: &ReviewRound,
        diff: &WorkspaceDiff,
    ) -> Result<(), FeatureError> {
        let roles = round.required_roles.iter().copied().filter(|role| {
            !feature.attempts.iter().any(|attempt| {
                attempt.claim.snapshot_id == Some(round.snapshot.id)
                    && attempt.claim.task.role == *role
            })
        });
        let (sender, mut finished) = mpsc::channel(4);
        let shared = Arc::new(feature.clone());
        let input = review_input(feature, diff)?;
        for role in roles {
            let task_id = self
                .codec
                .derived_id(round.snapshot.id, &format!("{role:?}"));
            let task = FeatureTask {
                id: task_id,
                role,
                title: review_title(role).into(),
                instructions: "Review the integrated snapshot and report concrete evidence.".into(),
                paths: Vec::new(),
                depends_on: Vec::new(),
                checks: Vec::new(),
            };
            let request = TaskRequest {
                operation_id: self.codec.derived_id(task_id, "initial-launch"),
                attempt_id: task_id,
                task,
                revision: Some(round.architecture.clone()),
                snapshot_id: Some(round.snapshot.id),
                base_oid: round.snapshot.git.head_oid.clone(),
                retry_of: None,
                input: input.clone(),
            };
            let owner = self.clone();
            let feature = shared.clone();
            let sender = sender.clone();
            self.conversations.spawn(Box::pin(async move {
                let outcome = owner.run_task(actor, &feature, request).await.map(|_| ());
                let _ = sender.send(outcome).await;
            }));
        }
        drop(sender);
        let mut outcome = Ok(());
        while let Some(result) = finished.recv().await {
            outcome = outcome.and(result);
        }
        outcome
    }

    async fn verify_integrated(
        &self,
        env: &Environment,
        project: &Project,
        feature: &FeatureState,
        round: &ReviewRound,
    ) -> Result<Vec<ReviewCheck>, FeatureError> {
        let tasks = plan_tasks(feature)?;
        let commands: BTreeSet<_> = tasks.iter().flat_map(|task| &task.checks).collect();
        let mut checks = Vec::new();
        for command in commands {
            let workspace = TaskWorkspace {
                feature_id: feature.feature.id,
                attempt_id: Uuid::new_v4(),
                base_oid: round.snapshot.git.head_oid.clone(),
                access: WorkspaceAccess::Write,
            };
            let passed = match self.workspaces.prepare_task(env, project, &workspace).await {
                Ok(()) => match self
                    .runtime
                    .run_check(env, project, &workspace, command)
                    .await
                {
                    Ok(result) => {
                        let acknowledged = self
                            .runtime
                            .acknowledge_check(env, project, &workspace)
                            .await;
                        result.exit_code == 0 && acknowledged.is_ok()
                    }
                    Err(_) => false,
                },
                Err(_) => false,
            };
            checks.push(ReviewCheck {
                label: command.clone(),
                passed,
            });
        }
        Ok(checks)
    }
}

fn collect_findings(
    codec: &dyn FeatureCodec,
    feature: &FeatureState,
    round: &ReviewRound,
) -> Result<Vec<ReviewFinding>, FeatureError> {
    let mut findings = Vec::new();
    let mut summaries = BTreeSet::new();
    for role in &round.required_roles {
        let attempt = feature
            .attempts
            .iter()
            .rev()
            .find(|attempt| {
                attempt.claim.snapshot_id == Some(round.snapshot.id)
                    && attempt.claim.task.role == *role
            })
            .filter(|attempt| attempt.status == AttemptStatus::Validated)
            .ok_or(FeatureError::ReviewBlocked)?;
        for finding in parse_reply(
            codec,
            attempt
                .result
                .as_deref()
                .ok_or(FeatureError::ReviewBlocked)?,
            &round.snapshot.git.patch_digest,
        )? {
            let summary = finding.summary.trim().to_owned();
            if summaries.insert((summary.clone(), finding.blocking)) {
                findings.push(ReviewFinding {
                    role: *role,
                    summary,
                    blocking: finding.blocking,
                });
            }
        }
    }
    Ok(findings)
}

fn scope_finding(
    feature: &FeatureState,
    diff: &WorkspaceDiff,
) -> Result<Option<ReviewFinding>, FeatureError> {
    let scopes: Vec<_> = plan_tasks(feature)?
        .iter()
        .flat_map(|task| &task.paths)
        .collect();
    let outside: Vec<_> = diff
        .changed_paths
        .iter()
        .filter(|path| {
            !scopes.iter().any(|scope| {
                scope.as_str() == path.as_str()
                    || scope.ends_with('/') && path.starts_with(scope.as_str())
            })
        })
        .collect();
    if outside.is_empty() {
        return Ok(None);
    }
    let examples = outside
        .iter()
        .take(8)
        .map(|path| path.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Ok(Some(ReviewFinding {
        role: TaskRole::GeneralReview,
        blocking: true,
        summary: format!(
            "{} changed path(s) are outside the current architecture approval: {}. Reopen the architecture to approve how to handle the earlier changes.",
            outside.len(),
            examples
        ),
    }))
}

fn review_roles(diff: &WorkspaceDiff) -> Vec<TaskRole> {
    let mut roles = vec![TaskRole::GeneralReview];
    let top_levels: BTreeSet<_> = diff
        .changed_paths
        .iter()
        .filter_map(|path| path.split('/').next())
        .collect();
    if top_levels.len() > 1
        || diff.changed_paths.iter().any(|path| {
            path.contains("architecture")
                || path.ends_with("Cargo.toml")
                || path.ends_with("package.json")
        })
    {
        roles.push(TaskRole::ArchitectureReview);
    }
    if diff.changed_paths.iter().any(|path| {
        path.rsplit_once('.').is_some_and(|(_, suffix)| {
            matches!(
                suffix,
                "html" | "jsx" | "tsx" | "css" | "scss" | "vue" | "svelte"
            )
        })
    }) {
        roles.push(TaskRole::WebReview);
    }
    let changed_lines = diff.patch.to_ascii_lowercase();
    if diff.changed_paths.iter().any(|path| {
        ["auth", "permission", "credential", "sandbox", "security"]
            .iter()
            .any(|term| path.to_ascii_lowercase().contains(term))
    }) || [
        "authorization",
        "csrf",
        "token",
        "sandbox",
        "secret",
        "permission",
    ]
    .iter()
    .any(|term| changed_lines.contains(term))
    {
        roles.push(TaskRole::SecurityReview);
    }
    roles
}

pub(super) fn review_input(
    feature: &FeatureState,
    diff: &WorkspaceDiff,
) -> Result<String, FeatureError> {
    let approved = approved_context(feature)?;
    let input = format!(
        "Review the immutable integrated change in /workspace/checkout. Inspect it with git -C /workspace/checkout diff --no-ext-diff --no-textconv {} {}. Check behavior against the approved brief and design below. Consider useful simplifications and include file and line evidence. Return exactly one JSON object: {{\"patchDigest\":\"{}\",\"coverage\":\"complete\",\"findings\":[{{\"summary\":\"path:line and concrete finding\",\"blocking\":true}}]}}. Use coverage partial if you could not inspect the relevant changes or lack decisive evidence. Use an empty findings array if the review found no verified issues. Do not rely on other agents' verdicts.\n\n{approved}",
        diff.git.base_oid, diff.git.head_oid, diff.git.patch_digest,
    );
    bounded_review_input(input)
}

pub(super) fn correction_request(
    codec: &dyn FeatureCodec,
    feature: &FeatureState,
    round: &ReviewRound,
    verdict: &ReviewVerdict,
) -> Result<TaskRequest, FeatureError> {
    let task_id = codec.derived_id(round.snapshot.id, "correction-task");
    let paths: BTreeSet<_> = plan_tasks(feature)?
        .iter()
        .flat_map(|task| task.paths.iter().cloned())
        .collect();
    let paths: Vec<_> = paths
        .iter()
        .filter(|path| {
            !paths
                .iter()
                .any(|scope| scope != *path && scope.ends_with('/') && path.starts_with(scope))
        })
        .cloned()
        .collect();
    if paths.is_empty() || paths.len() > 128 {
        return Err(FeatureError::ReviewBlocked);
    }
    let findings = verdict
        .findings
        .iter()
        .filter(|finding| finding.blocking)
        .map(|finding| finding.summary.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let failures = verdict
        .checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.label.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let approved = approved_context(feature)?;
    let input = bounded_review_input(format!(
        "Independently reproduce each reported blocker and failed check before editing. Fix confirmed problems inside the approved paths. Preserve approved behavior. If a change needs a product or architecture decision, report it without implementing it.\n\nReported blockers:\n{findings}\n\nChecks to investigate:\n{failures}\n\n{approved}"
    ))?;
    Ok(TaskRequest {
        operation_id: codec.derived_id(task_id, "initial-launch"),
        attempt_id: task_id,
        task: FeatureTask {
            id: task_id,
            title: "Review corrections".into(),
            instructions: "Resolve the integrated review blockers inside the approved paths."
                .into(),
            role: TaskRole::Correction,
            paths,
            depends_on: Vec::new(),
            checks: Vec::new(),
        },
        revision: Some(round.architecture.clone()),
        snapshot_id: Some(round.snapshot.id),
        base_oid: round.snapshot.git.head_oid.clone(),
        retry_of: None,
        input,
    })
}

fn bounded_review_input(input: String) -> Result<String, FeatureError> {
    if input.len() > 240_000 {
        Err(FeatureError::ReviewBlocked)
    } else {
        Ok(input)
    }
}

fn plan_tasks(feature: &FeatureState) -> Result<&[FeatureTask], FeatureError> {
    match &feature
        .current_architecture()
        .ok_or(FeatureError::Conflict)?
        .content
    {
        GateContent::Architecture { tasks, .. } => Ok(tasks),
        GateContent::Scope { .. } => Err(FeatureError::Conflict),
    }
}

fn review_title(role: TaskRole) -> &'static str {
    match role {
        TaskRole::ArchitectureReview => "Architecture review",
        TaskRole::SecurityReview => "Security review",
        TaskRole::WebReview => "Web review",
        _ => "General review",
    }
}

fn workspace_error(error: WorkspaceError) -> FeatureError {
    match error {
        WorkspaceError::Conflict => FeatureError::StaleSnapshot,
        WorkspaceError::Uncertain => FeatureError::ExecutionUncertain,
        WorkspaceError::Invalid | WorkspaceError::ScopeViolation => FeatureError::Invalid,
        WorkspaceError::Unavailable => FeatureError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_core::features::{FeatureProposal, FeatureReviewReply};

    struct DigestReply;

    impl FeatureCodec for DigestReply {
        fn parse_proposal(&self, _: &str) -> Result<FeatureProposal, FeatureError> {
            unreachable!()
        }
        fn revision_digest(&self, _: &GateContent) -> Result<String, FeatureError> {
            unreachable!()
        }
        fn render_revision(&self, _: &GateContent) -> Result<String, FeatureError> {
            unreachable!()
        }
        fn parse_review(&self, raw: &str) -> Result<FeatureReviewReply, FeatureError> {
            Ok(FeatureReviewReply {
                patch_digest: raw.trim_end_matches(":partial").into(),
                complete: !raw.ends_with(":partial"),
                findings: Vec::new(),
            })
        }
        fn derived_id(&self, _: Uuid, _: &str) -> Uuid {
            unreachable!()
        }
    }

    #[test]
    fn accepts_only_a_reply_for_the_reviewed_patch() {
        assert!(
            parse_reply(&DigestReply, "exact", "exact")
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            parse_reply(&DigestReply, "exact", "stale"),
            Err(FeatureError::StaleSnapshot)
        ));
        assert!(matches!(
            parse_reply(&DigestReply, "exact:partial", "exact"),
            Err(FeatureError::ReviewBlocked)
        ));
    }
}
