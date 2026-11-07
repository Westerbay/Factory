use super::*;

const GENERATION: i64 = 3;
const CHECK: &str = "cargo test --locked";

fn feature() -> FeatureState {
    FeatureState::new(Feature {
        id: Uuid::new_v4(),
        user_id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        conversation_id: Uuid::new_v4(),
        environment_id: Uuid::new_v4(),
        provider: "codex".into(),
        workflow_revision: "2026-11-07".into(),
        title: "Feature".into(),
        stage: FeatureStage::Scope,
        created_at: Utc::now(),
    })
}

fn apply(state: &mut FeatureState, command: FeatureCommand) -> Result<(), FeatureError> {
    apply_in(state, GENERATION, command)
}

fn apply_in(
    state: &mut FeatureState,
    generation: i64,
    command: FeatureCommand,
) -> Result<(), FeatureError> {
    state.apply(
        state.feature.user_id,
        generation,
        Uuid::new_v4(),
        command,
        Utc::now(),
    )
}

fn key() -> RevisionKey {
    RevisionKey {
        id: Uuid::new_v4(),
        digest: "f".repeat(64),
    }
}

fn propose_scope(state: &mut FeatureState, brief: &str) -> RevisionKey {
    let key = key();
    apply(
        state,
        FeatureCommand::ProposeRevision(GateRevision {
            key: key.clone(),
            content: GateContent::Scope {
                brief: brief.into(),
            },
            approval: None,
        }),
    )
    .unwrap();
    key
}

fn architecture(scope: RevisionKey, tasks: Vec<FeatureTask>) -> GateRevision {
    GateRevision {
        key: key(),
        content: GateContent::Architecture {
            scope,
            summary: "Keep boundaries separate".into(),
            diagrams: vec![FeatureDiagram {
                title: "Components".into(),
                source: "flowchart TD; API --> Service".into(),
            }],
            tasks,
        },
        approval: None,
    }
}

fn task(role: TaskRole, path: Option<&str>) -> FeatureTask {
    FeatureTask {
        id: Uuid::new_v4(),
        title: "Task".into(),
        instructions: "Perform scoped work.".into(),
        role,
        paths: path.into_iter().map(str::to_owned).collect(),
        depends_on: Vec::new(),
        checks: if path.is_some() {
            vec![CHECK.into()]
        } else {
            Vec::new()
        },
    }
}

fn approve_both(state: &mut FeatureState, tasks: Vec<FeatureTask>) -> RevisionKey {
    let scope = propose_scope(state, "What the member asked for");
    apply(state, FeatureCommand::ApproveRevision(scope.clone())).unwrap();
    let revision = architecture(scope, tasks);
    let key = revision.key.clone();
    apply(state, FeatureCommand::ProposeRevision(revision)).unwrap();
    apply(state, FeatureCommand::ApproveRevision(key.clone())).unwrap();
    key
}

fn claim(
    state: &FeatureState,
    task: FeatureTask,
    revision: Option<RevisionKey>,
    snapshot: Option<&ReviewSnapshot>,
) -> AttemptClaim {
    AttemptClaim {
        workspace: TaskWorkspace {
            feature_id: state.feature.id,
            attempt_id: Uuid::new_v4(),
            base_oid: snapshot.map_or_else(|| "a".repeat(40), |s| s.git.head_oid.clone()),
            access: task.role.access(),
        },
        task,
        revision,
        snapshot_id: snapshot.map(|s| s.id),
        model: "test-model".into(),
        effort: None,
        skill_revision: "test-skill".into(),
        retry_of: None,
    }
}

fn update(
    state: &mut FeatureState,
    attempt_id: Uuid,
    update: AttemptUpdate,
) -> Result<(), FeatureError> {
    apply(state, FeatureCommand::RecordAttempt { attempt_id, update })
}

fn launch(state: &mut FeatureState, claim: AttemptClaim) -> Uuid {
    let id = claim.workspace.attempt_id;
    apply(state, FeatureCommand::ClaimAttempt(claim)).unwrap();
    update(
        state,
        id,
        AttemptUpdate::LaunchSettled {
            process_receipt: Some("provider-runtime".into()),
        },
    )
    .unwrap();
    id
}

#[test]
fn new_product_revision_invalidates_prior_approval_without_erasing_history() {
    let mut state = feature();
    let discussion = claim(&state, task(TaskRole::Coordinator, None), None, None);
    let attempt = launch(&mut state, discussion);
    update(
        &mut state,
        attempt,
        AttemptUpdate::Validated { checkpoint: None },
    )
    .unwrap();

    let original = propose_scope(&mut state, "One viewer");
    let latest = propose_scope(&mut state, "Any invited viewer");
    assert_eq!(
        apply(&mut state, FeatureCommand::ApproveRevision(original)),
        Err(FeatureError::Conflict)
    );
    let approval = FeatureCommand::ApproveRevision(latest.clone());
    assert_eq!(
        state.apply(
            Uuid::new_v4(),
            GENERATION,
            Uuid::new_v4(),
            approval.clone(),
            Utc::now()
        ),
        Err(FeatureError::Unauthorized)
    );
    apply(&mut state, approval).unwrap();
    let revision = architecture(latest, vec![task(TaskRole::Implementation, Some("src/"))]);
    let approved = revision.key.clone();
    apply(&mut state, FeatureCommand::ProposeRevision(revision)).unwrap();
    apply(
        &mut state,
        FeatureCommand::ApproveRevision(approved.clone()),
    )
    .unwrap();

    let new_scope = propose_scope(&mut state, "Guests can edit too");
    assert_eq!(state.feature.stage, FeatureStage::Scope);
    assert_eq!(state.scope, Some(new_scope));
    assert_eq!(state.architecture, None);
    assert_eq!(
        apply(&mut state, FeatureCommand::ApproveRevision(approved)),
        Err(FeatureError::Conflict)
    );
    assert_eq!(
        state
            .revisions
            .iter()
            .filter(|r| r.approval.is_some())
            .count(),
        2
    );
}

#[test]
fn overlapping_writers_and_unknown_launches_block_retries() {
    let mut state = feature();
    let primary = task(TaskRole::Implementation, Some("src/"));
    let overlapping = task(TaskRole::ComplexImplementation, Some("src/lib.rs"));
    let independent = task(TaskRole::Implementation, Some("tests/check.rs"));
    let revision = approve_both(
        &mut state,
        vec![primary.clone(), overlapping.clone(), independent.clone()],
    );
    let original = claim(&state, primary, Some(revision.clone()), None);
    let attempt_id = original.workspace.attempt_id;
    apply(&mut state, FeatureCommand::ClaimAttempt(original.clone())).unwrap();
    let contested = claim(&state, overlapping, Some(revision.clone()), None);
    assert_eq!(
        apply(&mut state, FeatureCommand::ClaimAttempt(contested)),
        Err(FeatureError::Conflict)
    );
    let other = claim(&state, independent, Some(revision), None);
    let other_id = launch(&mut state, other);
    update(
        &mut state,
        other_id,
        AttemptUpdate::Validated {
            checkpoint: Some("c".repeat(40)),
        },
    )
    .unwrap();

    update(&mut state, attempt_id, AttemptUpdate::Uncertain).unwrap();
    let mut retry = original;
    retry.workspace.attempt_id = Uuid::new_v4();
    retry.retry_of = Some(attempt_id);
    assert_eq!(
        apply(&mut state, FeatureCommand::ClaimAttempt(retry.clone())),
        Err(FeatureError::ExecutionUncertain)
    );
    let recover = FeatureCommand::RecoverAttempt {
        attempt_id,
        launch_generation: GENERATION,
    };
    assert_eq!(
        apply_in(&mut state, GENERATION + 1, recover.clone()),
        Err(FeatureError::ExecutionUncertain)
    );
    assert_eq!(
        apply_in(
            &mut state,
            GENERATION + 1,
            FeatureCommand::RecordAttempt {
                attempt_id,
                update: AttemptUpdate::LaunchSettled {
                    process_receipt: None
                },
            }
        ),
        Err(FeatureError::Conflict)
    );
    update(
        &mut state,
        attempt_id,
        AttemptUpdate::LaunchSettled {
            process_receipt: None,
        },
    )
    .unwrap();
    apply_in(&mut state, GENERATION + 1, recover).unwrap();
    let mut wrong_base = retry.clone();
    wrong_base.workspace.base_oid = "d".repeat(40);
    assert_eq!(
        apply_in(
            &mut state,
            GENERATION + 1,
            FeatureCommand::ClaimAttempt(wrong_base)
        ),
        Err(FeatureError::Conflict)
    );
    apply_in(
        &mut state,
        GENERATION + 1,
        FeatureCommand::ClaimAttempt(retry),
    )
    .unwrap();
    assert_eq!(state.attempts[0].status, AttemptStatus::Interrupted);
    assert_eq!(state.attempts[1].status, AttemptStatus::Validated);
    assert_eq!(state.attempts[2].generation, GENERATION + 1);
}

#[test]
fn delivery_requires_review_evidence_for_exact_base_head_and_patch() {
    let mut state = feature();
    let writer = task(TaskRole::Implementation, Some("src/"));
    let revision = approve_both(&mut state, vec![writer.clone()]);
    let work = claim(&state, writer, Some(revision.clone()), None);
    let writer_id = launch(&mut state, work);
    update(
        &mut state,
        writer_id,
        AttemptUpdate::Validated {
            checkpoint: Some("b".repeat(40)),
        },
    )
    .unwrap();

    let snapshot = ReviewSnapshot {
        id: Uuid::new_v4(),
        git: GitSnapshot {
            base_oid: "a".repeat(40),
            head_oid: "b".repeat(40),
            patch_digest: "c".repeat(64),
        },
    };
    apply(
        &mut state,
        FeatureCommand::BeginReview {
            snapshot: snapshot.clone(),
            required_roles: vec![TaskRole::GeneralReview],
        },
    )
    .unwrap();
    let record_verdict = FeatureCommand::RecordReview {
        snapshot_id: snapshot.id,
        patch_digest: snapshot.git.patch_digest.clone(),
        verdict: ReviewVerdict {
            findings: Vec::new(),
            checks: vec![ReviewCheck {
                label: CHECK.into(),
                passed: true,
            }],
        },
    };
    assert_eq!(
        apply(&mut state, record_verdict.clone()),
        Err(FeatureError::ReviewBlocked)
    );
    let mut reviewer = claim(
        &state,
        task(TaskRole::GeneralReview, None),
        Some(revision),
        Some(&snapshot),
    );
    reviewer.workspace.access = WorkspaceAccess::Write;
    assert_eq!(
        apply(&mut state, FeatureCommand::ClaimAttempt(reviewer.clone())),
        Err(FeatureError::Invalid)
    );
    reviewer.workspace.access = WorkspaceAccess::ReadOnly;
    let reviewer_id = launch(&mut state, reviewer);
    assert_eq!(
        update(
            &mut state,
            reviewer_id,
            AttemptUpdate::Validated { checkpoint: None }
        ),
        Err(FeatureError::Conflict)
    );
    update(
        &mut state,
        reviewer_id,
        AttemptUpdate::Result {
            text: "No findings".into(),
        },
    )
    .unwrap();
    update(
        &mut state,
        reviewer_id,
        AttemptUpdate::Validated { checkpoint: None },
    )
    .unwrap();
    assert_eq!(
        apply(
            &mut state,
            FeatureCommand::RecordReview {
                snapshot_id: snapshot.id,
                patch_digest: "d".repeat(64),
                verdict: ReviewVerdict {
                    findings: Vec::new(),
                    checks: Vec::new()
                },
            }
        ),
        Err(FeatureError::StaleSnapshot)
    );
    assert_eq!(
        apply(&mut state, record_verdict.clone()),
        Err(FeatureError::ReviewBlocked)
    );
    complete_integrated_check(&mut state, snapshot.id, true);
    apply(&mut state, record_verdict).unwrap();
    assert_eq!(state.feature.stage, FeatureStage::Result);

    let mut moved = snapshot.git.clone();
    moved.base_oid = "d".repeat(40);
    assert_eq!(
        apply(
            &mut state,
            FeatureCommand::ClaimDelivery {
                snapshot_id: snapshot.id,
                observed: moved
            }
        ),
        Err(FeatureError::StaleSnapshot)
    );
    let publish = FeatureCommand::ClaimDelivery {
        snapshot_id: snapshot.id,
        observed: snapshot.git,
    };
    let request_id = Uuid::new_v4();
    state
        .apply(
            state.feature.user_id,
            GENERATION,
            request_id,
            publish.clone(),
            Utc::now(),
        )
        .unwrap();
    assert_eq!(
        apply(&mut state, publish),
        Err(FeatureError::ExecutionUncertain)
    );
    apply(
        &mut state,
        FeatureCommand::RecordDelivery {
            request_id,
            status: DeliveryStatus::Uncertain,
        },
    )
    .unwrap();
    apply(
        &mut state,
        FeatureCommand::RecordDelivery {
            request_id,
            status: DeliveryStatus::Published {
                draft_url: "https://example.com/reviewed-draft".into(),
            },
        },
    )
    .unwrap();
    assert_eq!(state.feature.stage, FeatureStage::Draft);
}

#[test]
fn an_integration_conflict_pins_the_retry_to_the_verified_branch_head() {
    let mut state = feature();
    let task = task(TaskRole::Implementation, Some("src/"));
    let revision = approve_both(&mut state, vec![task.clone()]);
    let original = claim(&state, task, Some(revision), None);
    let failed = launch(&mut state, original.clone());
    let head = "b".repeat(40);
    update(
        &mut state,
        failed,
        AttemptUpdate::IntegrationConflict {
            retry_base: head.clone(),
        },
    )
    .unwrap();
    assert_eq!(state.attempts[0].status, AttemptStatus::Failed);

    let mut retry = original;
    retry.workspace.attempt_id = Uuid::new_v4();
    retry.retry_of = Some(failed);
    assert_eq!(
        apply(&mut state, FeatureCommand::ClaimAttempt(retry.clone())),
        Err(FeatureError::Conflict)
    );
    retry.workspace.base_oid = head;
    apply(&mut state, FeatureCommand::ClaimAttempt(retry)).unwrap();
}

#[test]
fn retained_work_pins_retry_without_validating_or_erasing_the_failure() {
    let mut state = feature();
    let task = task(TaskRole::Implementation, Some("src/"));
    let revision = approve_both(&mut state, vec![task.clone()]);
    let original = claim(&state, task, Some(revision), None);
    let attempt_id = launch(&mut state, original.clone());
    let save = FeatureCommand::SaveRetryBase {
        attempt_id,
        base_oid: "c".repeat(40),
    };
    assert_eq!(apply(&mut state, save.clone()), Err(FeatureError::Conflict));
    let reason = "Verification failed";
    update(
        &mut state,
        attempt_id,
        AttemptUpdate::Failed {
            reason: reason.into(),
        },
    )
    .unwrap();

    apply(&mut state, save).unwrap();
    let prior = &state.attempts[0];
    assert_eq!(prior.status, AttemptStatus::Failed);
    assert_eq!(prior.activity.as_deref(), Some(reason));
    assert_eq!(
        prior.checkpoint.as_deref(),
        Some("cccccccccccccccccccccccccccccccccccccccc")
    );
    assert_eq!(
        apply(
            &mut state,
            FeatureCommand::SaveRetryBase {
                attempt_id,
                base_oid: "d".repeat(40),
            }
        ),
        Err(FeatureError::Conflict)
    );

    let mut retry = original;
    retry.workspace.attempt_id = Uuid::new_v4();
    retry.retry_of = Some(attempt_id);
    assert_eq!(
        apply(&mut state, FeatureCommand::ClaimAttempt(retry.clone())),
        Err(FeatureError::Conflict)
    );
    retry.workspace.base_oid = "c".repeat(40);
    let retry_id = launch(&mut state, retry.clone());
    update(&mut state, retry_id, AttemptUpdate::DiffUnrenderable).unwrap();
    assert_eq!(state.attempts[1].status, AttemptStatus::Failed);
    assert_eq!(
        state.attempts[1].activity.as_deref(),
        Some(WorkspaceError::DiffUnrenderable.to_string().as_str())
    );
    assert_eq!(
        apply(
            &mut state,
            FeatureCommand::SaveRetryBase {
                attempt_id: retry_id,
                base_oid: "d".repeat(40),
            },
        ),
        Err(FeatureError::Conflict)
    );

    retry.workspace.attempt_id = Uuid::new_v4();
    retry.retry_of = Some(retry_id);
    assert_eq!(
        apply(&mut state, FeatureCommand::ClaimAttempt(retry.clone())),
        Err(FeatureError::Conflict)
    );
    retry.workspace.base_oid = state.attempts[0].claim.workspace.base_oid.clone();
    apply(&mut state, FeatureCommand::ClaimAttempt(retry)).unwrap();
}

#[test]
fn observed_worker_proof_recovers_unknown_provider_and_check_launches() {
    let mut state = feature();
    let task = task(TaskRole::Implementation, Some("src/"));
    let revision = approve_both(&mut state, vec![task.clone()]);
    let original = claim(&state, task, Some(revision), None);
    let attempt_id = original.workspace.attempt_id;
    apply(&mut state, FeatureCommand::ClaimAttempt(original.clone())).unwrap();
    let recover = |attempt_id| FeatureCommand::RecoverAttempt {
        attempt_id,
        launch_generation: GENERATION,
    };
    let observed = |attempt_id| FeatureCommand::RecoverObservedAttempt {
        attempt_id,
        launch_generation: GENERATION,
        worker_id: "f".repeat(64),
    };
    assert_eq!(
        apply(&mut state, recover(attempt_id)),
        Err(FeatureError::ExecutionUncertain)
    );
    apply(&mut state, observed(attempt_id)).unwrap();
    assert_eq!(state.attempts[0].status, AttemptStatus::Interrupted);
    assert!(state.attempts[0].launch_settled);
    assert_eq!(
        state.attempts[0].process_receipt.as_ref(),
        Some(&"f".repeat(64))
    );

    let mut retry = original;
    retry.workspace.attempt_id = Uuid::new_v4();
    retry.retry_of = Some(attempt_id);
    let retry_id = launch(&mut state, retry);
    update(
        &mut state,
        retry_id,
        AttemptUpdate::VerificationStarted {
            workspace_id: Uuid::new_v4(),
        },
    )
    .unwrap();
    assert_eq!(
        update(
            &mut state,
            retry_id,
            AttemptUpdate::Validated {
                checkpoint: Some("a".repeat(40))
            },
        ),
        Err(FeatureError::Conflict)
    );
    update(&mut state, retry_id, AttemptUpdate::Uncertain).unwrap();
    assert_eq!(
        apply(&mut state, recover(retry_id)),
        Err(FeatureError::ExecutionUncertain)
    );
    apply(&mut state, observed(retry_id)).unwrap();
    assert_eq!(state.attempts[1].status, AttemptStatus::Interrupted);
    assert_eq!(state.attempts[1].pending_verification, None);
    assert_eq!(
        state.attempts[1].process_receipt.as_deref(),
        Some("provider-runtime")
    );
}

fn complete_integrated_check(state: &mut FeatureState, snapshot_id: Uuid, passed: bool) {
    let workspace_id = Uuid::new_v4();
    apply(
        state,
        FeatureCommand::ClaimIntegratedCheck {
            snapshot_id,
            workspace_id,
            command: CHECK.into(),
        },
    )
    .unwrap();
    apply(
        state,
        FeatureCommand::StartIntegratedCheck {
            snapshot_id,
            workspace_id,
        },
    )
    .unwrap();
    assert_eq!(
        apply(
            state,
            FeatureCommand::StartIntegratedCheck {
                snapshot_id,
                workspace_id
            },
        ),
        Err(FeatureError::Conflict)
    );
    apply(
        state,
        FeatureCommand::FinishIntegratedCheck {
            snapshot_id,
            workspace_id,
            passed,
        },
    )
    .unwrap();
}

#[test]
fn a_blocked_round_cannot_be_superseded_before_its_correction_or_with_the_same_patch() {
    let mut state = feature();
    let writer = task(TaskRole::Implementation, Some("src/"));
    let revision = approve_both(&mut state, vec![writer.clone()]);
    let initial = claim(&state, writer, Some(revision.clone()), None);
    let writer_id = launch(&mut state, initial);
    let original = ReviewSnapshot {
        id: Uuid::new_v4(),
        git: GitSnapshot {
            base_oid: "a".repeat(40),
            head_oid: "b".repeat(40),
            patch_digest: "c".repeat(64),
        },
    };
    update(
        &mut state,
        writer_id,
        AttemptUpdate::Validated {
            checkpoint: Some(original.git.head_oid.clone()),
        },
    )
    .unwrap();
    let begin = |snapshot: ReviewSnapshot| FeatureCommand::BeginReview {
        snapshot,
        required_roles: vec![TaskRole::GeneralReview],
    };
    apply(&mut state, begin(original.clone())).unwrap();
    let reviewer = claim(
        &state,
        task(TaskRole::GeneralReview, None),
        Some(revision.clone()),
        Some(&original),
    );
    let reviewer_id = launch(&mut state, reviewer);
    update(
        &mut state,
        reviewer_id,
        AttemptUpdate::Result {
            text: "Blocking issue".into(),
        },
    )
    .unwrap();
    update(
        &mut state,
        reviewer_id,
        AttemptUpdate::Validated { checkpoint: None },
    )
    .unwrap();
    complete_integrated_check(&mut state, original.id, true);
    apply(
        &mut state,
        FeatureCommand::RecordReview {
            snapshot_id: original.id,
            patch_digest: original.git.patch_digest.clone(),
            verdict: ReviewVerdict {
                findings: vec![ReviewFinding {
                    role: TaskRole::GeneralReview,
                    blocking: true,
                    summary: "Regression in src".into(),
                }],
                checks: vec![ReviewCheck {
                    label: CHECK.into(),
                    passed: true,
                }],
            },
        },
    )
    .unwrap();
    apply(
        &mut state,
        FeatureCommand::BeginCorrections {
            snapshot_id: original.id,
        },
    )
    .unwrap();

    let mut next = ReviewSnapshot {
        id: Uuid::new_v4(),
        git: original.git.clone(),
    };
    assert_eq!(
        apply(&mut state, begin(next.clone())),
        Err(FeatureError::ReviewBlocked)
    );
    let correction = claim(
        &state,
        task(TaskRole::Correction, Some("src/")),
        Some(revision),
        Some(&original),
    );
    let correction_id = launch(&mut state, correction);
    update(
        &mut state,
        correction_id,
        AttemptUpdate::Validated {
            checkpoint: Some("d".repeat(40)),
        },
    )
    .unwrap();
    assert_eq!(
        apply(&mut state, begin(next.clone())),
        Err(FeatureError::ReviewBlocked)
    );
    next.git.head_oid = "d".repeat(40);
    next.git.patch_digest = "e".repeat(64);
    apply(&mut state, begin(next.clone())).unwrap();
    assert_eq!(state.review().unwrap().snapshot, next);
}
