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
