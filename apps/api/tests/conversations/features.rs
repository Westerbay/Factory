use super::*;
use factory_adapters::feature_codec::JsonFeatureCodec;
use factory_application::features::{FeatureCoordinator, bundle::WORKFLOW_REVISION};
use factory_core::features::*;

struct Features {
    inner: Mutex<FeatureState>,
    receipts: Mutex<HashMap<Uuid, FeatureCommand>>,
}

#[async_trait]
impl FeatureStore for Features {
    async fn create(
        &self,
        _: EnvironmentActor,
        _: &Environment,
        _: NewFeature,
    ) -> Result<FeatureState, FeatureError> {
        unreachable!()
    }

    async fn list(
        &self,
        actor: EnvironmentActor,
        project_id: Option<Uuid>,
        _: Option<Uuid>,
    ) -> Result<FeaturePage, FeatureError> {
        let state = self.inner.lock().unwrap();
        let feature = &state.feature;
        let items = if feature.user_id == actor.user_id
            && project_id.is_none_or(|id| id == feature.project_id)
        {
            vec![feature.clone()]
        } else {
            vec![]
        };
        Ok(FeaturePage {
            items,
            next_cursor: None,
        })
    }

    async fn get(&self, actor: EnvironmentActor, id: Uuid) -> Result<FeatureState, FeatureError> {
        let state = self.inner.lock().unwrap();
        if state.feature.id == id && state.feature.user_id == actor.user_id {
            Ok(state.clone())
        } else {
            Err(FeatureError::NotFound)
        }
    }

    async fn get_by_conversation(
        &self,
        actor: EnvironmentActor,
        conversation_id: Uuid,
    ) -> Result<Option<FeatureState>, FeatureError> {
        let state = self.inner.lock().unwrap();
        Ok((state.feature.conversation_id == conversation_id
            && state.feature.user_id == actor.user_id)
            .then(|| state.clone()))
    }

    async fn apply(
        &self,
        actor: EnvironmentActor,
        env: &Environment,
        id: Uuid,
        operation_id: Uuid,
        command: FeatureCommand,
    ) -> Result<FeatureReceipt, FeatureError> {
        let mut state = self.inner.lock().unwrap();
        if state.feature.id != id || state.feature.user_id != actor.user_id {
            return Err(FeatureError::NotFound);
        }
        let mut receipts = self.receipts.lock().unwrap();
        if let Some(prior) = receipts.get(&operation_id) {
            if prior != &command {
                return Err(FeatureError::Conflict);
            }
            return Ok(FeatureReceipt {
                state: state.clone(),
                fresh: false,
            });
        }
        state.apply(
            actor.user_id,
            env.generation,
            operation_id,
            command.clone(),
            Utc::now(),
        )?;
        receipts.insert(operation_id, command);
        Ok(FeatureReceipt {
            state: state.clone(),
            fresh: true,
        })
    }
}

#[derive(Default)]
struct NoFeatureWorkers {
    initializations: AtomicUsize,
}

#[async_trait]
impl FeatureWorkspaceRuntime for NoFeatureWorkers {
    async fn initialize(
        &self,
        _: &Environment,
        _: &Project,
        _: Uuid,
    ) -> Result<FeatureBranch, WorkspaceError> {
        // Gate A may schedule architecture work. Keep this HTTP fixture inert.
        self.initializations.fetch_add(1, Ordering::SeqCst);
        Err(WorkspaceError::Unavailable)
    }

    async fn prepare_task(
        &self,
        _: &Environment,
        _: &Project,
        _: &TaskWorkspace,
    ) -> Result<(), WorkspaceError> {
        unreachable!()
    }

    async fn checkpoint(
        &self,
        _: &Environment,
        _: &Project,
        _: &TaskAttempt,
    ) -> Result<String, WorkspaceError> {
        unreachable!()
    }

    async fn integrate(
        &self,
        _: &Environment,
        _: &Project,
        _: &TaskAttempt,
        _: &str,
    ) -> Result<String, WorkspaceError> {
        unreachable!()
    }

    async fn snapshot(
        &self,
        _: &Environment,
        _: &Project,
        _: Uuid,
    ) -> Result<WorkspaceDiff, WorkspaceError> {
        unreachable!()
    }
}

#[async_trait]
impl FeatureExecutionRuntime for NoFeatureWorkers {
    async fn open_task(
        &self,
        _: &Environment,
        _: &Project,
        _: &FeatureRun,
    ) -> Result<FeatureExecution, ExecutionError> {
        unreachable!()
    }

    async fn run_check(
        &self,
        _: &Environment,
        _: &Project,
        _: &TaskWorkspace,
        _: &str,
    ) -> Result<FeatureCheckOutput, ExecutionError> {
        unreachable!()
    }

    async fn reconcile_task(
        &self,
        _: &Environment,
        _: &Project,
        _: &TaskAttempt,
    ) -> Result<TaskObservation, ExecutionError> {
        Ok(TaskObservation::Unknown)
    }
}

fn seed_feature(f: &ConversationFixture) -> (Environment, FeatureState, RevisionKey) {
    let env = f.store.environment.lock().unwrap().clone();
    let project_id = Uuid::new_v4();
    let conversation_id = Uuid::new_v4();
    let feature_id = Uuid::new_v4();
    let revision = RevisionKey {
        id: Uuid::new_v4(),
        digest: "b".repeat(64),
    };
    let now = Utc::now();
    *f.data.2.lock().unwrap() = Some(Project {
        id: project_id,
        user_id: env.user_id,
        repository_id: 42,
        full_name: "factory/fixture".into(),
        cloned_full_name: "factory/fixture".into(),
        directory: "fixture".into(),
        state: ProjectState::Ready,
        operation_id: Uuid::new_v4(),
        environment_id: env.id,
        environment_generation: env.generation,
        created_at: now,
        updated_at: now,
    });
    f.data.0.lock().unwrap().conversation = Some(Conversation {
        id: conversation_id,
        user_id: env.user_id,
        project_id: Some(project_id),
        provider_thread: None,
        created_at: now,
    });
    let mut state = FeatureState::new(Feature {
        id: feature_id,
        user_id: env.user_id,
        project_id,
        conversation_id,
        environment_id: env.id,
        provider: "codex".into(),
        workflow_revision: WORKFLOW_REVISION.into(),
        title: "Test feature".into(),
        stage: FeatureStage::Scope,
        created_at: now,
    });
    state
        .apply(
            env.user_id,
            env.generation,
            Uuid::new_v4(),
            FeatureCommand::ProposeRevision(GateRevision {
                key: revision.clone(),
                content: GateContent::Scope {
                    brief: "Test scope".into(),
                },
                approval: None,
            }),
            now,
        )
        .unwrap();
    (env, state, revision)
}

fn feature_router<W>(
    f: &ConversationFixture,
    store: Arc<Features>,
    provider: Arc<dyn ExecutionRuntime>,
    workers: Arc<W>,
) -> Router
where
    W: FeatureExecutionRuntime + FeatureWorkspaceRuntime + 'static,
{
    let accounts = Arc::new(AgentConnectionService::new(
        f.environments.clone(),
        Arc::new(Connected(true)),
    ));
    let conversations = factory_api::conversations::runtime::with_providers_and_features(
        f.conversation_service.clone(),
        vec![ExecutionProvider {
            id: "codex",
            display_name: "Codex",
            accounts,
            runtime: provider,
        }],
        store.clone(),
    )
    .unwrap();
    let coordinator = FeatureCoordinator::new(
        store.clone(),
        conversations.clone(),
        workers.clone(),
        workers,
        Arc::new(JsonFeatureCodec),
        Arc::new(FixtureRepositoryVerifier),
    );
    conversations::router(f.auth.clone(), Some(conversations)).merge(factory_api::features::router(
        f.auth.clone(),
        coordinator,
        None,
    ))
}

struct FixtureRepositoryVerifier;

#[async_trait]
impl FeatureRepositoryVerifier for FixtureRepositoryVerifier {
    async fn verify(
        &self,
        actor: EnvironmentActor,
        environment: &Environment,
        project: &Project,
    ) -> Result<(), FeatureError> {
        assert_eq!(actor.user_id, project.user_id);
        assert_eq!(environment.id, project.environment_id);
        Ok(())
    }
}

#[tokio::test]
async fn feature_gate_requires_ownership_and_csrf_and_replays_the_exact_command() {
    let mut f = ConversationFixture::new().await;
    let (_, state, revision) = seed_feature(&f);
    let feature_id = state.feature.id;
    let conversation_id = state.feature.conversation_id;
    let store = Arc::new(Features {
        inner: Mutex::new(state),
        receipts: Mutex::default(),
    });
    f.app = feature_router(
        &f,
        store.clone(),
        f.provider.clone(),
        Arc::new(NoFeatureWorkers::default()),
    );

    let path = format!("/api/features/{feature_id}");
    let detail = f.call("GET", &path, 0, "").await;
    assert_eq!(detail.status(), StatusCode::OK);
    let detail = body_json(detail).await;
    assert_eq!(detail["scope"]["canApprove"], true);
    assert_eq!(
        detail["feature"]["conversationId"],
        conversation_id.to_string()
    );
    assert_eq!(
        f.call("GET", &path, 1, "").await.status(),
        StatusCode::NOT_FOUND
    );

    let approve = format!("{path}/approve");
    let operation = Uuid::new_v4();
    let body =
        json!({"operationId": operation, "revisionId": revision.id, "digest": revision.digest});
    let no_csrf = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&approve)
                .header("cookie", format!("factory_session={}", f.tokens[0]))
                .header("origin", ORIGIN)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_csrf.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        f.call("POST", &approve, 1, &body.to_string())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(store.receipts.lock().unwrap().is_empty());

    for _ in 0..2 {
        let approved = f.call("POST", &approve, 0, &body.to_string()).await;
        assert_eq!(approved.status(), StatusCode::OK);
        let view = body_json(approved).await;
        assert_eq!(view["feature"]["stage"], "architecture");
        assert_eq!(view["scope"]["canApprove"], false);
        assert!(view["scope"]["approvedAt"].is_string());
    }
    assert_eq!(store.receipts.lock().unwrap().len(), 1);
    let changed =
        json!({"operationId": operation, "revisionId": revision.id, "digest": "c".repeat(64)});
    assert_eq!(
        f.call("POST", &approve, 0, &changed.to_string())
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let generic_turn = f.call(
        "POST",
        &format!("/api/conversations/{conversation_id}/runs"),
        0,
        &json!({"operationId":Uuid::new_v4(), "model":"factory:codex:fixture-model", "input":"hello"}).to_string(),
    ).await;
    assert_eq!(generic_turn.status(), StatusCode::CONFLICT);
    assert_eq!(f.provider.opens.load(Ordering::SeqCst), 0);
}

struct FeatureCatalog;

#[tokio::test]
async fn feature_continue_wakes_stranded_gate_and_requires_proof_for_orphaned_attempt() {
    let mut f = ConversationFixture::new().await;
    let (env, mut state, scope) = seed_feature(&f);
    let feature_id = state.feature.id;
    let now = Utc::now();
    state
        .apply(
            env.user_id,
            env.generation,
            Uuid::new_v4(),
            FeatureCommand::ApproveRevision(scope.clone()),
            now,
        )
        .unwrap();
    let store = Arc::new(Features {
        inner: Mutex::new(state),
        receipts: Mutex::default(),
    });
    let workers = Arc::new(NoFeatureWorkers::default());
    f.app = feature_router(&f, store.clone(), Arc::new(FeatureCatalog), workers.clone());
    let path = format!("/api/features/{feature_id}");
    let before = body_json(f.call("GET", &path, 0, "").await).await;
    assert_eq!(before["canContinue"], true);
    assert_eq!(before["feature"]["stage"], "architecture");
    assert_eq!(workers.initializations.load(Ordering::SeqCst), 0);
    let resume = format!("{path}/resume");
    let continued = f
        .call(
            "POST",
            &resume,
            0,
            &json!({"operationId": Uuid::new_v4()}).to_string(),
        )
        .await;
    assert_eq!(continued.status(), StatusCode::OK);
    until(|| workers.initializations.load(Ordering::SeqCst) == 1).await;
    let continued = body_json(continued).await;
    assert_eq!(continued["scope"]["id"], before["scope"]["id"]);
    assert_eq!(
        continued["scope"]["approvedAt"],
        before["scope"]["approvedAt"]
    );

    let attempt_id = Uuid::new_v4();
    store
        .inner
        .lock()
        .unwrap()
        .apply(
            env.user_id,
            env.generation,
            Uuid::new_v4(),
            FeatureCommand::ClaimAttempt(AttemptClaim {
                task: FeatureTask {
                    id: Uuid::new_v4(),
                    title: "Architecture".into(),
                    instructions: "Prepare the architecture".into(),
                    role: TaskRole::Architecture,
                    paths: vec![],
                    depends_on: vec![],
                    checks: vec![],
                },
                revision: Some(scope),
                workspace: TaskWorkspace {
                    feature_id,
                    attempt_id,
                    base_oid: test_oid(0),
                    access: WorkspaceAccess::ReadOnly,
                },
                snapshot_id: None,
                model: "gpt-6-astra".into(),
                effort: None,
                skill_revision: WORKFLOW_REVISION.into(),
                retry_of: None,
            }),
            now,
        )
        .unwrap();
    let orphaned = body_json(f.call("GET", &path, 0, "").await).await;
    assert_eq!(orphaned["canContinue"], false);
    assert_eq!(orphaned["attempts"][0]["canResume"], true);
    let cannot_prove_stop = f
        .call(
            "POST",
            &resume,
            0,
            &json!({"operationId": Uuid::new_v4(), "attemptId": attempt_id}).to_string(),
        )
        .await;
    assert_eq!(cannot_prove_stop.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(cannot_prove_stop).await["code"],
        "feature-execution-uncertain"
    );
    assert_eq!(store.inner.lock().unwrap().attempts.len(), 1);
}

#[async_trait]
impl ExecutionRuntime for FeatureCatalog {
    async fn models(&self, _: &Environment) -> Result<Vec<ExecutionModel>, ExecutionError> {
        Ok(["gpt-6.1-terra", "gpt-6.1-sol"]
            .into_iter()
            .map(|model| ExecutionModel {
                id: model.into(),
                name: model.into(),
                resolved_model: Some(model.into()),
                effort_levels: vec!["medium".into(), "high".into()],
                default_effort: Some("medium".into()),
            })
            .collect())
    }

    async fn open(
        &self,
        _: &Environment,
        _: Option<&str>,
        _: &str,
        _: Option<&str>,
        _: &str,
    ) -> Result<ExecutionSession, ExecutionError> {
        panic!("a feature must not use the unrestricted conversation launcher")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkerAction {
    Alpha,
    Beta,
    Review,
    Correction,
}

#[derive(Default)]
struct WorkflowObservations {
    base: usize,
    launches: Vec<(Uuid, WorkerAction, String)>,
    integrated: Vec<Uuid>,
    checks: Vec<(String, i64)>,
}

impl WorkflowObservations {
    fn head(&self) -> usize {
        self.base + self.integrated.len()
    }
}

#[derive(Default)]
struct WorkflowWorkers {
    observed: Mutex<WorkflowObservations>,
    alpha_check: Notify,
}

fn test_oid(index: usize) -> String {
    format!("{:040x}", index + 1)
}

fn test_digest(index: usize) -> String {
    format!("{:064x}", index + 1)
}

#[async_trait]
impl FeatureWorkspaceRuntime for WorkflowWorkers {
    async fn initialize(
        &self,
        _: &Environment,
        _: &Project,
        _: Uuid,
    ) -> Result<FeatureBranch, WorkspaceError> {
        Ok(FeatureBranch {
            base_ref: "refs/heads/main".into(),
            base_oid: test_oid(0),
            head_ref: "refs/heads/fixture".into(),
        })
    }

    async fn prepare_task(
        &self,
        _: &Environment,
        _: &Project,
        _: &TaskWorkspace,
    ) -> Result<(), WorkspaceError> {
        Ok(())
    }

    async fn checkpoint(
        &self,
        _: &Environment,
        _: &Project,
        attempt: &TaskAttempt,
    ) -> Result<String, WorkspaceError> {
        let observed = self.observed.lock().unwrap();
        let position = observed
            .launches
            .iter()
            .position(|(id, _, _)| *id == attempt.id())
            .unwrap();
        Ok(test_oid(100 + position))
    }

    async fn integrate(
        &self,
        _: &Environment,
        _: &Project,
        attempt: &TaskAttempt,
        expected_head: &str,
    ) -> Result<String, WorkspaceError> {
        let mut observed = self.observed.lock().unwrap();
        if expected_head != test_oid(observed.head()) {
            return Err(WorkspaceError::Conflict);
        }
        assert!(!observed.integrated.contains(&attempt.id()));
        observed.integrated.push(attempt.id());
        Ok(test_oid(observed.head()))
    }

    async fn snapshot(
        &self,
        _: &Environment,
        _: &Project,
        _: Uuid,
    ) -> Result<WorkspaceDiff, WorkspaceError> {
        let observed = self.observed.lock().unwrap();
        let head = observed.head();
        Ok(WorkspaceDiff {
            git: GitSnapshot {
                base_oid: test_oid(observed.base),
                head_oid: test_oid(head),
                patch_digest: test_digest(head),
            },
            patch: format!("Fixture diff at integration {head}"),
            changed_paths: vec!["src/alpha.rs".into(), "src/beta.rs".into()],
        })
    }
}

#[async_trait]
impl FeatureExecutionRuntime for WorkflowWorkers {
    async fn open_task(
        &self,
        _: &Environment,
        _: &Project,
        run: &FeatureRun,
    ) -> Result<FeatureExecution, ExecutionError> {
        let action = if run.workspace.access == WorkspaceAccess::ReadOnly {
            WorkerAction::Review
        } else if run.input.contains("Task: Alpha writer") {
            WorkerAction::Alpha
        } else if run.input.contains("Task: Beta writer") {
            WorkerAction::Beta
        } else {
            assert!(run.input.contains("fixture defect"));
            WorkerAction::Correction
        };
        let (first_beta, head) = {
            let mut observed = self.observed.lock().unwrap();
            let first_beta = action == WorkerAction::Beta
                && observed
                    .launches
                    .iter()
                    .all(|(_, action, _)| *action != WorkerAction::Beta);
            observed
                .launches
                .push((run.workspace.attempt_id, action, run.model.clone()));
            (first_beta, observed.head())
        };
        let mut events = vec![
            ExecutionEvent::ThreadReady {
                provider_thread: "private-fixture-thread".into(),
            },
            ExecutionEvent::ModelObserved {
                model: run.model.clone(),
            },
        ];
        if action == WorkerAction::Review {
            let digest = test_digest(head);
            assert!(run.input.contains(&digest));
            let findings = if head == 2 {
                vec![json!({"summary": "fixture defect", "blocking": true})]
            } else {
                vec![]
            };
            events.push(ExecutionEvent::Message {
                provider_item: "valid".into(),
                text: json!({"patchDigest": digest, "coverage": "complete", "findings": findings})
                    .to_string(),
            });
            if head == 2 {
                events.push(ExecutionEvent::Message {
                    provider_item: "stale".into(),
                    text: json!({"patchDigest": test_digest(0), "coverage": "complete", "findings": []}).to_string(),
                });
                events.push(ExecutionEvent::Message {
                    provider_item: "partial".into(),
                    text: json!({"patchDigest": digest, "coverage": "partial", "findings": []})
                        .to_string(),
                });
            }
        }
        events.push(if first_beta {
            ExecutionEvent::Failed
        } else {
            ExecutionEvent::Completed
        });
        let (tx, rx) = mpsc::channel(events.len());
        for event in events {
            tx.try_send(event).unwrap();
        }
        Ok(FeatureExecution {
            process_receipt: "private-fixture-process".into(),
            session: ExecutionSession {
                control: Box::new(Control {
                    started: Arc::default(),
                    stopped: Arc::default(),
                }),
                output: Box::new(Output(rx)),
            },
        })
    }

    async fn run_check(
        &self,
        _: &Environment,
        _: &Project,
        workspace: &TaskWorkspace,
        command: &str,
    ) -> Result<FeatureCheckOutput, ExecutionError> {
        let is_integrated_check = self
            .observed
            .lock()
            .unwrap()
            .launches
            .iter()
            .all(|(id, _, _)| *id != workspace.attempt_id);
        if !is_integrated_check && command == "check-alpha" {
            self.alpha_check.notified().await;
        }
        let exit_code = if is_integrated_check
            && workspace.base_oid == test_oid(2)
            && command == "check-beta"
        {
            7
        } else {
            0
        };
        self.observed
            .lock()
            .unwrap()
            .checks
            .push((command.to_owned(), exit_code));
        Ok(FeatureCheckOutput {
            exit_code,
            output: "fixture evidence".into(),
        })
    }

    async fn reconcile_task(
        &self,
        _: &Environment,
        _: &Project,
        _: &TaskAttempt,
    ) -> Result<TaskObservation, ExecutionError> {
        Ok(TaskObservation::Stopped)
    }
}

fn feature_task(title: &str, path: &str, check: &str) -> FeatureTask {
    FeatureTask {
        id: Uuid::new_v4(),
        title: title.into(),
        instructions: format!("Implement {title}"),
        role: TaskRole::Implementation,
        paths: vec![path.into()],
        depends_on: vec![],
        checks: vec![check.into()],
    }
}

#[tokio::test]
async fn feature_resumes_only_the_failed_writer_and_reviews_the_corrected_snapshot() {
    let mut f = ConversationFixture::new().await;
    let (env, mut state, scope) = seed_feature(&f);
    let feature_id = state.feature.id;
    let architecture = RevisionKey {
        id: Uuid::new_v4(),
        digest: "c".repeat(64),
    };
    let alpha = feature_task("Alpha writer", "src/alpha.rs", "check-alpha");
    let beta = feature_task("Beta writer", "src/beta.rs", "check-beta");
    let critique_id = Uuid::new_v4();
    let critique = FeatureTask {
        id: Uuid::new_v4(),
        title: "Architecture critique".into(),
        instructions: "Review design".into(),
        role: TaskRole::ArchitectureReview,
        paths: vec![],
        depends_on: vec![],
        checks: vec![],
    };
    for command in [
        FeatureCommand::ApproveRevision(scope.clone()),
        FeatureCommand::ProposeRevision(GateRevision {
            key: architecture.clone(),
            content: GateContent::Architecture {
                scope,
                summary: "Two disjoint implementation tasks".into(),
                diagrams: vec![FeatureDiagram {
                    title: "Flow".into(),
                    source: "flowchart LR\nA --> B".into(),
                }],
                tasks: vec![alpha.clone(), beta.clone()],
            },
            approval: None,
        }),
        FeatureCommand::ClaimAttempt(AttemptClaim {
            task: critique,
            revision: Some(architecture.clone()),
            workspace: TaskWorkspace {
                feature_id,
                attempt_id: critique_id,
                base_oid: test_oid(0),
                access: WorkspaceAccess::ReadOnly,
            },
            snapshot_id: None,
            model: "gpt-6-astra".into(),
            effort: None,
            skill_revision: WORKFLOW_REVISION.into(),
            retry_of: None,
        }),
        FeatureCommand::RecordAttempt {
            attempt_id: critique_id,
            update: AttemptUpdate::LaunchSettled {
                process_receipt: Some("private-critique-process".into()),
            },
        },
        FeatureCommand::RecordAttempt {
            attempt_id: critique_id,
            update: AttemptUpdate::Result {
                text: "Design verified".into(),
            },
        },
        FeatureCommand::RecordAttempt {
            attempt_id: critique_id,
            update: AttemptUpdate::Validated { checkpoint: None },
        },
    ] {
        state
            .apply(
                env.user_id,
                env.generation,
                Uuid::new_v4(),
                command,
                Utc::now(),
            )
            .unwrap();
    }
    let store = Arc::new(Features {
        inner: Mutex::new(state),
        receipts: Mutex::default(),
    });
    let workers = Arc::new(WorkflowWorkers::default());
    f.app = feature_router(&f, store.clone(), Arc::new(FeatureCatalog), workers.clone());
    let path = format!("/api/features/{feature_id}");
    let approved = f
        .call(
            "POST",
            &format!("{path}/approve"),
            0,
            &json!({
                "operationId": Uuid::new_v4(),
                "revisionId": architecture.id,
                "digest": architecture.digest,
            })
            .to_string(),
        )
        .await;
    assert_eq!(approved.status(), StatusCode::OK);

    until(|| {
        let state = store.inner.lock().unwrap();
        state
            .attempts
            .iter()
            .any(|a| a.claim.task.id == alpha.id && a.status == AttemptStatus::Running)
            && state
                .attempts
                .iter()
                .any(|a| a.claim.task.id == beta.id && a.status == AttemptStatus::Failed)
    })
    .await;
    let before = body_json(f.call("GET", &path, 0, "").await).await;
    let attempts = before["attempts"].as_array().unwrap();
    let active = attempts
        .iter()
        .find(|item| item["title"] == alpha.title)
        .unwrap();
    assert_eq!(active["canResume"], false);
    let active_resume = f
        .call(
            "POST",
            &format!("{path}/resume"),
            0,
            &json!({"operationId": Uuid::new_v4(), "attemptId": active["id"]}).to_string(),
        )
        .await;
    assert_eq!(active_resume.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(active_resume).await["code"],
        "feature-execution-uncertain"
    );
    workers.alpha_check.notify_one();
    until(|| {
        store
            .inner
            .lock()
            .unwrap()
            .attempts
            .iter()
            .any(|a| a.claim.task.id == alpha.id && a.status == AttemptStatus::Validated)
    })
    .await;
    let failed = attempts
        .iter()
        .find(|item| item["title"] == beta.title)
        .unwrap();
    assert_eq!(failed["canResume"], true);
    assert_eq!(failed["observedModel"], "gpt-6.1-terra");
    assert!(failed.get("processReceipt").is_none());
    assert!(before["architecture"].get("tasks").is_none());
    let resumed = f
        .call(
            "POST",
            &format!("{path}/resume"),
            0,
            &json!({
                "operationId": Uuid::new_v4(),
                "attemptId": failed["id"],
            })
            .to_string(),
        )
        .await;
    assert_eq!(resumed.status(), StatusCode::OK);
    until(|| store.inner.lock().unwrap().feature.stage == FeatureStage::Result).await;

    let detail = body_json(f.call("GET", &path, 0, "").await).await;
    let diff = body_json(f.call("GET", &format!("{path}/diff"), 0, "").await).await;
    assert_eq!(detail["review"]["ready"], true);
    assert_eq!(detail["review"]["patchDigest"], diff["patchDigest"]);
    assert_eq!(diff["patchDigest"], test_digest(3));
    assert_eq!(
        detail["review"]["checks"],
        json!([
            {"label":"check-alpha","passed":true},
            {"label":"check-beta","passed":true},
        ])
    );
    let reviewed_snapshot = &detail["review"]["snapshotId"];
    let review_path = format!("{path}/review");
    let review_request =
        || json!({"operationId": Uuid::new_v4(), "snapshotId": reviewed_snapshot}).to_string();
    let unchanged = f.call("POST", &review_path, 0, &review_request()).await;
    assert_eq!(unchanged.status(), StatusCode::CONFLICT);
    {
        let state = store.inner.lock().unwrap();
        assert_eq!(state.feature.stage, FeatureStage::Result);
        assert_eq!(state.reviews.len(), 2);
        assert_eq!(state.additional_review_rounds, 0);
    }

    // Observe a moved base through the workspace port. This does not run or
    // integrate another writer, but the previously reviewed diff is stale.
    workers.observed.lock().unwrap().base = 1;
    let moved_diff = body_json(f.call("GET", &format!("{path}/diff"), 0, "").await).await;
    assert_eq!(moved_diff["patchDigest"], test_digest(4));
    assert_eq!(moved_diff["baseOid"], test_oid(1));
    let changed_request = review_request();
    let requested = f.call("POST", &review_path, 0, &changed_request).await;
    assert_eq!(requested.status(), StatusCode::OK);
    assert_eq!(body_json(requested).await["feature"]["stage"], "review");
    until(|| {
        let state = store.inner.lock().unwrap();
        state.feature.stage == FeatureStage::Result && state.reviews.len() == 3
    })
    .await;

    let reviewed = body_json(f.call("GET", &path, 0, "").await).await;
    assert_eq!(reviewed["review"]["ready"], true);
    assert_ne!(&reviewed["review"]["snapshotId"], reviewed_snapshot);
    assert_eq!(reviewed["review"]["patchDigest"], moved_diff["patchDigest"]);
    assert_eq!(reviewed["review"]["headOid"], moved_diff["headOid"]);
    assert_eq!(reviewed["review"]["baseOid"], moved_diff["baseOid"]);
    let replay = f.call("POST", &review_path, 0, &changed_request).await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(
        body_json(replay).await["review"]["snapshotId"],
        reviewed["review"]["snapshotId"]
    );
    let stale = f.call("POST", &review_path, 0, &review_request()).await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(stale).await["code"], "feature-stale-snapshot");

    {
        let observed = workers.observed.lock().unwrap();
        for (action, count, model) in [
            (WorkerAction::Alpha, 1, "gpt-6.1-terra"),
            (WorkerAction::Beta, 2, "gpt-6.1-terra"),
            (WorkerAction::Correction, 1, "gpt-6.1-terra"),
            (WorkerAction::Review, 3, "gpt-6.1-sol"),
        ] {
            let launches: Vec<_> = observed
                .launches
                .iter()
                .filter(|(_, recorded, _)| *recorded == action)
                .collect();
            assert_eq!(launches.len(), count, "{action:?}");
            assert!(launches.iter().all(|(_, _, actual)| actual == model));
        }
        assert!(observed.checks.contains(&("check-beta".into(), 7)));
        assert_eq!(observed.integrated.len(), 3);
    }
    {
        let mut state = store.inner.lock().unwrap();
        assert_eq!(state.reviews.len(), 3);
        assert_eq!(state.additional_review_rounds, 1);
        let first = state.reviews[0].verdict.as_ref().unwrap();
        assert!(
            first
                .findings
                .iter()
                .any(|finding| finding.blocking && finding.summary == "fixture defect")
        );
        assert!(
            first
                .checks
                .iter()
                .any(|check| check.label == "check-beta" && !check.passed)
        );

        // Emulate a persisted stale GitHub receipt. This router has no delivery
        // adapter, so it must not show the earlier local patch as a PR diff.
        let snapshot = state.review().unwrap().snapshot.clone();
        let request_id = Uuid::new_v4();
        state
            .apply(
                env.user_id,
                env.generation,
                request_id,
                FeatureCommand::ClaimDelivery {
                    snapshot_id: snapshot.id,
                    observed: snapshot.git,
                },
                Utc::now(),
            )
            .unwrap();
        state
            .apply(
                env.user_id,
                env.generation,
                Uuid::new_v4(),
                FeatureCommand::RecordDelivery {
                    request_id,
                    status: DeliveryStatus::PublishedStale {
                        draft_url: "https://github.com/factory/fixture/pull/1".into(),
                    },
                },
                Utc::now(),
            )
            .unwrap();
    }
    let unavailable_remote_diff = f.call("GET", &format!("{path}/diff"), 0, "").await;
    assert_eq!(
        unavailable_remote_diff.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}
