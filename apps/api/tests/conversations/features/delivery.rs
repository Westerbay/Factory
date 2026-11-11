use super::*;
use factory_application::{features::FeatureDeliveryService, identity::RepositoryAccessService};

struct DeliveryGit {
    workers: Arc<WorkflowWorkers>,
    remote: Mutex<RemoteObservation>,
}

#[derive(Default)]
struct RemoteObservation {
    base: usize,
    head: Option<String>,
    managed: Option<String>,
    move_base_after_push: bool,
    lose_push_proof: bool,
    advanced: bool,
    refresh_failures: usize,
    replace_on_stage: bool,
    replace_on_lookup: Option<usize>,
    replaced: bool,
    staged: Option<(Uuid, GitSnapshot)>,
    confirmed: usize,
    pushes: Vec<GitSnapshot>,
}

#[async_trait]
impl FeatureDeliveryRuntime for DeliveryGit {
    async fn remote(
        &self,
        _: &Environment,
        _: &Project,
        _: Uuid,
    ) -> Result<FeatureRemoteRefs, WorkspaceError> {
        let mut state = self.remote.lock().unwrap();
        let observed = FeatureRemoteRefs {
            base_oid: Some(test_oid(state.base)),
            head_oid: state.head.clone(),
            managed_head_oid: state.managed.clone(),
        };
        if state.move_base_after_push && state.pushes.len() == 1 && !state.advanced {
            // The push confirmation read succeeds. The next observation, just
            // before the draft POST, sees the advanced default branch.
            state.base = 5;
            state.advanced = true;
        }
        Ok(observed)
    }

    async fn push_reviewed(
        &self,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
        approved: &GitSnapshot,
    ) -> Result<(), WorkspaceError> {
        assert_eq!(
            self.workers.snapshot(env, project, feature_id).await?.git,
            *approved
        );
        let mut state = self.remote.lock().unwrap();
        assert_eq!(test_oid(state.base), approved.base_oid);
        assert_eq!(state.head, state.managed);
        if let Some(previous) = state.head.as_ref() {
            assert_eq!(previous, &state.pushes[0].head_oid);
            assert_ne!(previous, &approved.head_oid);
        }
        state.head = Some(approved.head_oid.clone());
        state.pushes.push(approved.clone());
        if state.lose_push_proof {
            return Err(WorkspaceError::Uncertain);
        }
        state.managed = state.head.clone();
        Ok(())
    }

    async fn stage_refresh(
        &self,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
        candidate_id: Uuid,
        approved: &GitSnapshot,
    ) -> Result<GitSnapshot, WorkspaceError> {
        assert_eq!(
            self.workers.snapshot(env, project, feature_id).await?.git,
            *approved
        );
        let integrated = self.workers.observed.lock().unwrap().integrated.len();
        let mut state = self.remote.lock().unwrap();
        assert_eq!(state.head.as_deref(), Some(approved.head_oid.as_str()));
        assert_eq!(state.managed, state.head);
        if state.refresh_failures > 0 {
            state.refresh_failures -= 1;
            return Err(WorkspaceError::Conflict);
        }
        let proposed = GitSnapshot {
            base_oid: test_oid(state.base),
            head_oid: test_oid(state.base + integrated),
            patch_digest: test_digest(state.base + integrated),
        };
        state.staged = Some((candidate_id, proposed.clone()));
        state.replaced = state.replace_on_stage;
        Ok(proposed)
    }

    async fn confirm_refresh(
        &self,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
        candidate_id: Uuid,
        approved: &GitSnapshot,
        proposed: &GitSnapshot,
    ) -> Result<WorkspaceDiff, WorkspaceError> {
        assert_eq!(
            self.workers.snapshot(env, project, feature_id).await?.git,
            *approved
        );
        let new_base = {
            let mut state = self.remote.lock().unwrap();
            assert_eq!(
                state.staged.as_ref(),
                Some(&(candidate_id, proposed.clone()))
            );
            assert!(!state.replaced);
            state.confirmed += 1;
            state.base
        };
        self.workers.observed.lock().unwrap().base = new_base;
        self.workers.snapshot(env, project, feature_id).await
    }
}

struct DraftFixture {
    git: Arc<DeliveryGit>,
    posts: Mutex<Vec<RepositoryDraft>>,
    lookups: AtomicUsize,
}

#[async_trait]
impl RepositoryAccessStore for DraftFixture {
    async fn begin_repository_authorization(
        &self,
        _: RepositoryAuthorizationAttempt,
    ) -> Result<(), RepositoryAccessError> {
        unreachable!()
    }
    async fn take_repository_authorization(
        &self,
        _: CredentialHash,
        _: CredentialHash,
        _: CredentialHash,
    ) -> Result<RepositoryAuthorizationAttempt, RepositoryAccessError> {
        unreachable!()
    }
    async fn validate_repository_epoch(
        &self,
        _: RepositoryActor,
        epoch: i64,
    ) -> Result<(), RepositoryAccessError> {
        assert_eq!(epoch, 1);
        Ok(())
    }
    async fn verify_repository_identity(
        &self,
        _: RepositoryActor,
        _: &VerifiedIdentity,
        _: i64,
    ) -> Result<(), RepositoryAccessError> {
        unreachable!()
    }
    async fn save_repository_credential(
        &self,
        _: RepositoryActor,
        _: &VerifiedIdentity,
        _: RepositoryCredential,
    ) -> Result<(), RepositoryAccessError> {
        unreachable!()
    }
    async fn repository_credential(
        &self,
        _: RepositoryActor,
    ) -> Result<Option<RepositoryCredential>, RepositoryAccessError> {
        Ok(Some(RepositoryCredential {
            subject: "42".into(),
            login: "fixture".into(),
            email: "fixture@example.test".into(),
            encrypted: EncryptedRepositoryToken {
                key_id: "test".into(),
                nonce: vec![],
                ciphertext: vec![],
            },
            authorization_epoch: 1,
        }))
    }
    async fn rotate_repository_token(
        &self,
        _: RepositoryActor,
        _: i64,
        _: &EncryptedRepositoryToken,
        _: EncryptedRepositoryToken,
    ) -> Result<(), RepositoryAccessError> {
        unreachable!()
    }
    async fn delete_repository_credential(
        &self,
        _: RepositoryActor,
    ) -> Result<(), RepositoryAccessError> {
        unreachable!()
    }
}

#[async_trait]
impl RepositoryAccessProvider for DraftFixture {
    fn authorization_url(&self, _: &str, _: &str) -> Result<String, RepositoryAccessError> {
        unreachable!()
    }
    async fn exchange(
        &self,
        _: &str,
        _: &str,
    ) -> Result<(RepositoryAccessToken, VerifiedIdentity), RepositoryAccessError> {
        unreachable!()
    }
    async fn primary_email(
        &self,
        _: &RepositoryAccessToken,
        _: &dyn RepositoryOperationGuard,
    ) -> Result<String, RepositoryAccessError> {
        unreachable!()
    }
    async fn repositories(
        &self,
        _: &RepositoryAccessToken,
        _: u32,
    ) -> Result<AccessibleRepositoryPage, RepositoryAccessError> {
        unreachable!()
    }
    async fn verify_clone_origin(
        &self,
        _: &RepositoryAccessToken,
        repo: i64,
        full_name: &str,
    ) -> Result<RepositoryOriginVerification, RepositoryAccessError> {
        assert_eq!((repo, full_name), (42, "factory/fixture"));
        Ok(if self.git.remote.lock().unwrap().replaced {
            RepositoryOriginVerification::Changed
        } else {
            RepositoryOriginVerification::Matched
        })
    }
    async fn public_key_registered(
        &self,
        _: &RepositoryAccessToken,
        _: &str,
        _: &dyn RepositoryOperationGuard,
    ) -> Result<bool, RepositoryAccessError> {
        unreachable!()
    }
    async fn ensure_public_key(
        &self,
        _: &RepositoryAccessToken,
        _: &str,
        _: &dyn RepositoryOperationGuard,
    ) -> Result<(), RepositoryAccessError> {
        unreachable!()
    }
    async fn find_draft(
        &self,
        _: &RepositoryAccessToken,
        _: &RepositoryDraftTarget<'_>,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<Option<RepositoryDraft>, RepositoryAccessError> {
        guard.check().await?;
        let count = self.lookups.fetch_add(1, Ordering::SeqCst) + 1;
        let mut remote = self.git.remote.lock().unwrap();
        if remote.replace_on_lookup == Some(count) {
            remote.replaced = true;
        }
        Ok(self.posts.lock().unwrap().last().cloned())
    }
    async fn create_draft(
        &self,
        _: &RepositoryAccessToken,
        target: &RepositoryDraftTarget<'_>,
        _: &str,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<RepositoryDraft, RepositoryAccessError> {
        guard.check().await?;
        let observed = self.git.remote.lock().unwrap();
        assert_eq!(observed.head.as_deref(), Some(target.head_oid));
        let draft = RepositoryDraft {
            url: "https://github.com/factory/fixture/pull/1".into(),
            base_oid: test_oid(observed.base),
            head_oid: target.head_oid.into(),
        };
        self.posts.lock().unwrap().push(draft.clone());
        Ok(draft)
    }
}

impl RepositoryTokenCipher for DraftFixture {
    fn seal(
        &self,
        _: Uuid,
        _: &RepositoryAccessToken,
    ) -> Result<EncryptedRepositoryToken, RepositoryAccessError> {
        unreachable!()
    }
    fn open(
        &self,
        _: Uuid,
        _: &EncryptedRepositoryToken,
    ) -> Result<RepositoryAccessToken, RepositoryAccessError> {
        RepositoryAccessToken::new("fixture-token".into())
    }
}

#[async_trait]
impl RepositoryPublicKeySource for DraftFixture {
    async fn public_key_status(
        &self,
        _: RepositoryActor,
    ) -> Result<RepositoryPublicKey, RepositoryAccessError> {
        unreachable!()
    }
    async fn mark_registered(
        &self,
        _: RepositoryActor,
        _: &str,
    ) -> Result<(), RepositoryAccessError> {
        unreachable!()
    }
    async fn rotate_missing_key(
        &self,
        _: RepositoryActor,
        _: &str,
    ) -> Result<RepositoryPublicKey, RepositoryAccessError> {
        unreachable!()
    }
}

fn ready_for_publication(
    f: &ConversationFixture,
    workers: &WorkflowWorkers,
) -> (FeatureState, ReviewSnapshot) {
    let (env, mut state, scope) = seed_feature(f);
    let architecture = RevisionKey {
        id: Uuid::new_v4(),
        digest: "a".repeat(64),
    };
    let task = feature_task("Completed change", "src/", "check-alpha");
    let attempt_id = Uuid::new_v4();
    let mut apply = |command| {
        state
            .apply(
                env.user_id,
                env.generation,
                Uuid::new_v4(),
                command,
                Utc::now(),
            )
            .unwrap();
    };
    apply(FeatureCommand::ApproveRevision(scope.clone()));
    apply(FeatureCommand::ProposeRevision(GateRevision {
        key: architecture.clone(),
        content: GateContent::Architecture {
            scope,
            summary: "Test delivery architecture".into(),
            diagrams: vec![FeatureDiagram {
                title: "Flow".into(),
                source: "flowchart LR\nA --> B".into(),
            }],
            tasks: vec![task.clone()],
        },
        approval: None,
    }));
    apply(FeatureCommand::ApproveRevision(architecture.clone()));
    let feature_id = state.feature.id;
    state.attempts.push(TaskAttempt {
        claim: AttemptClaim {
            task,
            revision: Some(architecture.clone()),
            workspace: TaskWorkspace {
                feature_id,
                attempt_id,
                base_oid: test_oid(0),
                access: WorkspaceAccess::Write,
            },
            snapshot_id: None,
            model: "gpt-6.1-terra".into(),
            effort: None,
            skill_revision: WORKFLOW_REVISION.into(),
            retry_of: None,
        },
        generation: env.generation,
        status: AttemptStatus::Validated,
        launch_settled: true,
        process_receipt: None,
        provider_thread: None,
        observed_model: None,
        activity: None,
        result: None,
        checkpoint: Some(test_oid(1)),
        pending_verification: None,
    });
    workers.observed.lock().unwrap().integrated.push(attempt_id);
    let reviewed = ReviewSnapshot {
        id: Uuid::new_v4(),
        git: GitSnapshot {
            base_oid: test_oid(0),
            head_oid: test_oid(1),
            patch_digest: test_digest(1),
        },
    };
    state.feature.stage = FeatureStage::Result;
    state.current_review = Some(reviewed.id);
    state.reviews.push(ReviewRound {
        snapshot: reviewed.clone(),
        architecture,
        required_roles: vec![TaskRole::GeneralReview],
        verdict: Some(ReviewVerdict {
            findings: vec![],
            checks: vec![ReviewCheck {
                label: "check-alpha".into(),
                passed: true,
            }],
        }),
        integrated_checks: vec![],
    });
    (state, reviewed)
}

#[tokio::test]
async fn feature_rechecks_a_base_advance_after_push_before_opening_a_draft() {
    exercise_moved_base(false).await;
}

#[tokio::test]
async fn feature_can_retry_a_failed_local_refresh_after_the_base_advances() {
    exercise_moved_base(true).await;
}

struct PublicationFixture {
    f: ConversationFixture,
    workers: Arc<WorkflowWorkers>,
    store: Arc<Features>,
    git: Arc<DeliveryGit>,
    provider: Arc<DraftFixture>,
    reviewed: ReviewSnapshot,
    path: String,
}

async fn publication_fixture(remote: RemoteObservation) -> PublicationFixture {
    let mut f = ConversationFixture::new().await;
    let workers = Arc::new(WorkflowWorkers::default());
    let (state, reviewed) = ready_for_publication(&f, &workers);
    let id = state.feature.id;
    let store = Arc::new(Features::new(state));
    let git = Arc::new(DeliveryGit {
        workers: workers.clone(),
        remote: Mutex::new(remote),
    });
    let provider = Arc::new(DraftFixture {
        git: git.clone(),
        posts: Mutex::default(),
        lookups: AtomicUsize::default(),
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
            runtime: Arc::new(FeatureCatalog),
        }],
        store.clone(),
    )
    .unwrap();
    let coordinator = FeatureCoordinator::new(
        store.clone(),
        conversations,
        workers.clone(),
        workers.clone(),
        Arc::new(JsonFeatureCodec),
        Arc::new(FixtureRepositoryVerifier),
    );
    let repository = Arc::new(RepositoryAccessService::new(
        provider.clone(),
        provider.clone(),
        provider.clone(),
        provider.clone(),
    ));
    let service = FeatureDeliveryService::new(coordinator.clone(), git.clone(), repository);
    f.app = factory_api::features::router(f.auth.clone(), coordinator, Some(service));
    PublicationFixture {
        f,
        workers,
        store,
        git,
        provider,
        reviewed,
        path: format!("/api/features/{id}"),
    }
}

fn draft_body(operation_id: Uuid, snapshot_id: Uuid) -> String {
    json!({"snapshotId": snapshot_id, "operationId": operation_id}).to_string()
}

async fn exercise_moved_base(retry_refresh: bool) {
    let PublicationFixture {
        f,
        store,
        git,
        provider,
        reviewed,
        path,
        ..
    } = publication_fixture(RemoteObservation {
        move_base_after_push: true,
        refresh_failures: usize::from(retry_refresh),
        ..Default::default()
    })
    .await;
    let draft_path = format!("{path}/draft");
    let draft = |snapshot_id: Uuid| draft_body(Uuid::new_v4(), snapshot_id);

    // An exact but unowned remote SHA is insufficient to authorize either write.
    git.remote.lock().unwrap().head = Some(reviewed.git.head_oid.clone());
    let conflict = f.call("POST", &draft_path, 0, &draft(reviewed.id)).await;
    assert_eq!(conflict.status(), StatusCode::OK);
    assert_eq!(body_json(conflict).await["delivery"]["status"], "failed");
    assert!(git.remote.lock().unwrap().pushes.is_empty());
    assert!(provider.posts.lock().unwrap().is_empty());
    git.remote.lock().unwrap().head = None;

    let stale = f.call("POST", &draft_path, 0, &draft(reviewed.id)).await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(stale).await["code"], "feature-stale-snapshot");
    assert_eq!(
        git.remote.lock().unwrap().pushes.as_slice(),
        std::slice::from_ref(&reviewed.git)
    );
    assert!(provider.posts.lock().unwrap().is_empty());

    let obsolete = f.call("POST", &draft_path, 0, &draft(reviewed.id)).await;
    assert_eq!(obsolete.status(), StatusCode::CONFLICT);
    assert!(provider.posts.lock().unwrap().is_empty());
    if retry_refresh {
        let still_previous = f.call("GET", &format!("{path}/diff"), 0, "").await;
        assert_eq!(
            body_json(still_previous).await["baseOid"],
            reviewed.git.base_oid
        );
        let response = f
            .call("POST", &format!("{path}/review"), 0, &draft(reviewed.id))
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    let diff = f.call("GET", &format!("{path}/diff"), 0, "").await;
    assert_eq!(diff.status(), StatusCode::OK);
    let diff = body_json(diff).await;
    assert_eq!(diff["baseOid"], test_oid(5));
    assert_eq!(diff["headOid"], test_oid(6));
    assert_ne!(diff["patchDigest"], reviewed.git.patch_digest);
    until(|| {
        let state = store.inner.lock().unwrap();
        state.feature.stage == FeatureStage::Result
            && state.current_review.is_some_and(|id| id != reviewed.id)
    })
    .await;
    let new_review = store
        .inner
        .lock()
        .unwrap()
        .review()
        .unwrap()
        .snapshot
        .clone();
    let published = f.call("POST", &draft_path, 0, &draft(new_review.id)).await;
    assert_eq!(published.status(), StatusCode::OK);
    assert_eq!(
        body_json(published).await["delivery"]["status"],
        "published"
    );
    assert_eq!(
        git.remote.lock().unwrap().pushes,
        [reviewed.git, new_review.git.clone()]
    );
    let posts = provider.posts.lock().unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].base_oid, new_review.git.base_oid);
    assert_eq!(posts[0].head_oid, new_review.git.head_oid);
}

#[tokio::test]
async fn feature_settles_a_lost_push_receipt_without_adopting_the_remote_branch() {
    let PublicationFixture {
        f,
        store,
        git,
        provider,
        reviewed,
        path,
        ..
    } = publication_fixture(RemoteObservation {
        lose_push_proof: true,
        ..Default::default()
    })
    .await;
    let draft_path = format!("{path}/draft");
    let operation = Uuid::new_v4();
    let request = draft_body(operation, reviewed.id);
    let first = f.call("POST", &draft_path, 0, &request).await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(body_json(first).await["delivery"]["status"], "uncertain");

    // A moved branch might conceal an in-flight draft, so it is not settled.
    git.remote.lock().unwrap().head = Some(test_oid(9));
    let moved = f.call("POST", &draft_path, 0, &request).await;
    assert_eq!(moved.status(), StatusCode::OK);
    assert_eq!(body_json(moved).await["delivery"]["status"], "uncertain");

    // A replacement namespace may not supply evidence of draft absence.
    {
        let mut remote = git.remote.lock().unwrap();
        remote.head = Some(reviewed.git.head_oid.clone());
        remote.replace_on_lookup = Some(provider.lookups.load(Ordering::SeqCst) + 2);
    }
    let replaced = f.call("POST", &draft_path, 0, &request).await;
    assert_eq!(replaced.status(), StatusCode::OK);
    assert_eq!(body_json(replaced).await["delivery"]["status"], "uncertain");
    {
        let mut remote = git.remote.lock().unwrap();
        remote.replaced = false;
        remote.replace_on_lookup = None;
    }

    // An exact, stable, unproved head and guarded draft absence settle the claim.
    let lookups_before = provider.lookups.load(Ordering::SeqCst);
    let settled = f.call("POST", &draft_path, 0, &request).await;
    assert_eq!(settled.status(), StatusCode::OK);
    let settled = body_json(settled).await;
    assert_eq!(settled["delivery"]["status"], "failed");
    assert_eq!(settled["delivery"]["requestId"], operation.to_string());
    assert!(
        settled["delivery"]["reason"]
            .as_str()
            .unwrap()
            .contains("ownership")
    );
    assert_eq!(provider.lookups.load(Ordering::SeqCst) - lookups_before, 2);
    assert!(matches!(
        store.inner.lock().unwrap().deliveries[0].status,
        DeliveryStatus::Failed { .. }
    ));

    // A repeat returns the exact terminal receipt; a new request cannot adopt it.
    let repeated = f.call("POST", &draft_path, 0, &request).await;
    assert_eq!(body_json(repeated).await["delivery"], settled["delivery"]);
    let new = f
        .call(
            "POST",
            &draft_path,
            0,
            &draft_body(Uuid::new_v4(), reviewed.id),
        )
        .await;
    assert_eq!(body_json(new).await["delivery"]["status"], "failed");
    let remote = git.remote.lock().unwrap();
    assert_eq!(
        remote.pushes.as_slice(),
        std::slice::from_ref(&reviewed.git)
    );
    assert_eq!(remote.head.as_deref(), Some(reviewed.git.head_oid.as_str()));
    assert_eq!(remote.managed, None);
    assert!(provider.posts.lock().unwrap().is_empty());
}

#[tokio::test]
async fn feature_does_not_review_a_staged_refresh_after_repository_identity_changes() {
    let PublicationFixture {
        f,
        workers,
        store,
        git,
        provider,
        reviewed,
        path,
    } = publication_fixture(RemoteObservation {
        move_base_after_push: true,
        replace_on_stage: true,
        ..Default::default()
    })
    .await;
    let stale = f
        .call(
            "POST",
            &format!("{path}/draft"),
            0,
            &draft_body(Uuid::new_v4(), reviewed.id),
        )
        .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(stale).await["code"], "feature-stale-snapshot");
    let diff = f.call("GET", &format!("{path}/diff"), 0, "").await;
    let diff = body_json(diff).await;
    assert_eq!(diff["baseOid"], reviewed.git.base_oid);
    assert_eq!(diff["headOid"], reviewed.git.head_oid);
    assert_eq!(diff["patchDigest"], reviewed.git.patch_digest);
    let observed = store.inner.lock().unwrap();
    assert_eq!(observed.feature.stage, FeatureStage::Review);
    assert_eq!(observed.current_review, Some(reviewed.id));
    let remote = git.remote.lock().unwrap();
    assert!(remote.replaced && remote.staged.is_some());
    assert_eq!(remote.confirmed, 0);
    assert!(workers.observed.lock().unwrap().launches.is_empty());
    assert!(provider.posts.lock().unwrap().is_empty());
}
