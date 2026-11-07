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

struct NoFeatureWorkers;

#[async_trait]
impl FeatureWorkspaceRuntime for NoFeatureWorkers {
    async fn initialize(
        &self,
        _: &Environment,
        _: &Project,
        _: Uuid,
    ) -> Result<FeatureBranch, WorkspaceError> {
        // Gate A may schedule architecture work. Keep this HTTP fixture inert.
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
        unreachable!()
    }
}

#[tokio::test]
async fn feature_gate_requires_ownership_and_csrf_and_replays_the_exact_command() {
    let mut f = ConversationFixture::new().await;
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
    let store = Arc::new(Features {
        inner: Mutex::new(state),
        receipts: Mutex::default(),
    });
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
            runtime: f.provider.clone(),
        }],
        store.clone(),
    )
    .unwrap();
    let coordinator = FeatureCoordinator::new(
        store.clone(),
        conversations.clone(),
        Arc::new(NoFeatureWorkers),
        Arc::new(NoFeatureWorkers),
        Arc::new(JsonFeatureCodec),
    );
    f.app = conversations::router(f.auth.clone(), Some(conversations)).merge(
        factory_api::features::router(f.auth.clone(), coordinator, None),
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
