use super::*;
use crate::{auth::AuthState, config::RuntimeConfig};
use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, header},
};
use chrono::{Duration, Utc};
use factory_application::identity::IdentityService;
use factory_core::identity::*;
use tower::ServiceExt;

struct Store {
    hash: CredentialHash,
    session: ActiveSession,
}
#[async_trait]
impl IdentityStore for Store {
    async fn create_login_attempt(&self, _: NewLoginAttempt) -> Result<(), IdentityError> {
        Err(IdentityError::Unavailable)
    }
    async fn take_login_attempt(
        &self,
        _: CredentialHash,
        _: CredentialHash,
    ) -> Result<LoginAttempt, IdentityError> {
        Err(IdentityError::Unavailable)
    }
    async fn admit(&self, _: LoginAdmission) -> Result<AdmissionOutcome, IdentityError> {
        Err(IdentityError::Unavailable)
    }
    async fn active_session(
        &self,
        hash: CredentialHash,
    ) -> Result<Option<ActiveSession>, IdentityError> {
        Ok((hash == self.hash).then(|| self.session.clone()))
    }
    async fn logout(&self, _: CredentialHash) -> Result<(), IdentityError> {
        Ok(())
    }
    async fn list_users(
        &self,
        _: CredentialHash,
        _: UserListQuery,
    ) -> Result<UserPage, IdentityError> {
        Err(IdentityError::Unavailable)
    }
    async fn change_access(
        &self,
        _: CredentialHash,
        _: AccessRequest,
    ) -> Result<User, IdentityError> {
        Err(IdentityError::Unavailable)
    }
}
struct Provider;
#[async_trait]
impl IdentityProvider for Provider {
    fn provider(&self) -> &str {
        "github"
    }
    fn issuer(&self) -> &str {
        "https://github.com"
    }
    fn authorization_url(&self, _: &str, _: &str) -> Result<String, IdentityError> {
        Err(IdentityError::Unavailable)
    }
    async fn exchange(&self, _: &str, _: &str) -> Result<VerifiedIdentity, IdentityError> {
        Err(IdentityError::Unavailable)
    }
}
fn fixture() -> (Router, String) {
    let raw = random_token().unwrap();
    let hash = hash_token(&raw).unwrap();
    let session = ActiveSession {
        user: User {
            id: Uuid::new_v4(),
            display_name: "Fixture".into(),
            handle: "fixture".into(),
            provider: "github".into(),
            role: Role::Member,
            status: AccessStatus::Active,
            version: 1,
            access_enabled_at: Some(Utc::now()),
        },
        csrf_token: "fixture-csrf".into(),
        expires_at: Utc::now() + Duration::hours(1),
    };
    let identity = Arc::new(IdentityService::new(
        Arc::new(Store { hash, session }),
        Arc::new(Provider),
    ));
    let auth = AuthState::new(identity, &RuntimeConfig::from_values(|_| None).unwrap());
    (
        documented_router(Some(auth), None).into(),
        format!("factory_session={raw}"),
    )
}
#[tokio::test]
async fn project_mutations_require_session_origin_and_csrf_even_when_runtime_is_missing() {
    let (app, cookie) = fixture();
    for path in ["/api/projects/connect", "/api/projects/setup"] {
        for (cookie_header, origin, csrf, expected) in [
            ("", true, true, StatusCode::UNAUTHORIZED),
            (cookie.as_str(), false, true, StatusCode::FORBIDDEN),
            (cookie.as_str(), true, false, StatusCode::FORBIDDEN),
            (cookie.as_str(), true, true, StatusCode::SERVICE_UNAVAILABLE),
        ] {
            let mut req = Request::builder().method("POST").uri(path);
            if !cookie_header.is_empty() {
                req = req.header(header::COOKIE, cookie_header)
            }
            if origin {
                req = req.header(header::ORIGIN, "http://localhost:3000")
            }
            if csrf {
                req = req.header("x-csrf-token", "fixture-csrf")
            }
            let response = app
                .clone()
                .oneshot(req.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
    }
}
#[tokio::test]
async fn deleting_project_requires_session_origin_and_csrf() {
    let (app, cookie) = fixture();
    let path = format!("/api/projects/{}", Uuid::new_v4());
    for (cookie_header, origin, csrf, expected) in [
        ("", true, true, StatusCode::UNAUTHORIZED),
        (cookie.as_str(), false, true, StatusCode::FORBIDDEN),
        (cookie.as_str(), true, false, StatusCode::FORBIDDEN),
        (cookie.as_str(), true, true, StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let mut req = Request::builder().method("DELETE").uri(&path);
        if !cookie_header.is_empty() {
            req = req.header(header::COOKIE, cookie_header);
        }
        if origin {
            req = req.header(header::ORIGIN, "http://localhost:3000");
        }
        if csrf {
            req = req.header("x-csrf-token", "fixture-csrf");
        }
        assert_eq!(
            app.clone()
                .oneshot(req.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            expected
        );
    }
}

#[tokio::test]
async fn provider_revocation_does_not_look_like_a_factory_session_revocation() {
    let response = access_error(RepositoryAccessError::ProviderUnauthorized);
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "provider_unauthorized");
    for error in [
        RepositoryAccessError::ProviderForbidden,
        RepositoryAccessError::Forbidden,
        RepositoryAccessError::InvalidLogin,
    ] {
        assert_eq!(access_error(error).status(), StatusCode::CONFLICT);
    }
}

#[tokio::test]
async fn imports_reject_browser_selected_names_and_recovery_fails_closed() {
    let (app, cookie) = fixture();
    let request = Request::builder()
        .method("POST")
        .uri("/api/projects")
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, "http://localhost:3000")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-csrf-token", "fixture-csrf")
        .body(Body::from(
            r#"{"repositoryId":7,"page":1,"fullName":"other/repo"}"#,
        ))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    let request = Request::builder()
        .method("GET")
        .uri("/api/projects/recoverable")
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn repository_metadata_sync_has_no_get_route_and_requires_csrf() {
    let (app, cookie) = fixture();
    let get = Request::builder()
        .method("GET")
        .uri("/api/projects/repositories")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(get).await.unwrap().status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    for (csrf, expected) in [
        (false, StatusCode::FORBIDDEN),
        (true, StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/projects/repositories")
            .header(header::COOKIE, &cookie)
            .header(header::ORIGIN, "http://localhost:3000")
            .header(header::CONTENT_TYPE, "application/json");
        if csrf {
            request = request.header("x-csrf-token", "fixture-csrf");
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(r#"{"page":1}"#)).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
}

#[test]
fn picker_excludes_reserved_repository_ids_without_losing_the_next_page() {
    let page = AccessibleRepositoryPage {
        items: vec![
            AccessibleRepository {
                id: 10,
                full_name: "old/name".into(),
                private: true,
            },
            AccessibleRepository {
                id: 11,
                full_name: "new/project".into(),
                private: false,
            },
        ],
        next_page: Some(2),
    };
    let view = importable(page, &std::collections::HashSet::from([10]));
    assert_eq!(view.items.len(), 1);
    assert_eq!(view.items[0].id, 11);
    assert_eq!(view.next_page, Some(2));
}

#[test]
fn project_projection_flags_provider_rename_without_claiming_remote_inspection() {
    use factory_core::projects::{Project, ProjectState};
    let id = Uuid::new_v4();
    let mut project = Project {
        id,
        user_id: Uuid::new_v4(),
        repository_id: 7,
        full_name: "owner/repo".into(),
        cloned_full_name: "owner/repo".into(),
        directory: format!("project-{id}"),
        state: ProjectState::Ready,
        operation_id: Uuid::new_v4(),
        environment_id: Uuid::new_v4(),
        environment_generation: 1,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let before = serde_json::to_value(ProjectView::from(project.clone())).unwrap();
    assert_eq!(before["renamedSinceClone"], false);
    project.full_name = "new-owner/new-name".into();
    let after = serde_json::to_value(ProjectView::from(project)).unwrap();
    assert_eq!(after["renamedSinceClone"], true);
    assert_eq!(after["fullName"], "new-owner/new-name");
    assert!(after.get("clonedFullName").is_none());
    assert!(after.get("directory").is_none());
    assert!(after.get("origin").is_none());
}

struct RecoveryEnvironment {
    owner: Uuid,
    hash: CredentialHash,
    value: std::sync::Mutex<factory_core::environments::Environment>,
}
#[async_trait]
impl factory_core::environments::EnvironmentStore for RecoveryEnvironment {
    async fn get(
        &self,
        a: EnvironmentActor,
    ) -> Result<
        Option<factory_core::environments::Environment>,
        factory_core::environments::EnvironmentError,
    > {
        if a.user_id != self.owner || a.session_hash != self.hash {
            return Err(factory_core::environments::EnvironmentError::Unauthorized);
        }
        Ok(Some(self.value.lock().unwrap().clone()))
    }
    async fn initialize_if_absent(
        &self,
        _: EnvironmentActor,
        _: factory_core::environments::EnvironmentStart,
    ) -> Result<factory_core::environments::Environment, factory_core::environments::EnvironmentError>
    {
        unreachable!()
    }
    async fn start(
        &self,
        _: EnvironmentActor,
        _: factory_core::environments::EnvironmentStart,
    ) -> Result<factory_core::environments::Environment, factory_core::environments::EnvironmentError>
    {
        unreachable!()
    }
    async fn stop(
        &self,
        _: EnvironmentActor,
    ) -> Result<
        Option<factory_core::environments::Environment>,
        factory_core::environments::EnvironmentError,
    > {
        unreachable!()
    }
    async fn reconcile_batch(
        &self,
        _: Uuid,
        _: u32,
    ) -> Result<
        Vec<factory_core::environments::Environment>,
        factory_core::environments::EnvironmentError,
    > {
        Ok(vec![])
    }
    async fn begin_provisioning(
        &self,
        _: Uuid,
        _: i64,
    ) -> Result<bool, factory_core::environments::EnvironmentError> {
        unreachable!()
    }
    async fn record_observation(
        &self,
        _: Uuid,
        _: i64,
        _: factory_core::environments::EnvironmentObservation,
        _: u32,
    ) -> Result<bool, factory_core::environments::EnvironmentError> {
        unreachable!()
    }
}
struct NoRuntime;
#[async_trait]
impl factory_core::environments::EnvironmentRuntime for NoRuntime {
    async fn inspect(
        &self,
        _: &factory_core::environments::Environment,
    ) -> Result<
        factory_core::environments::RuntimeInspection,
        factory_core::environments::EnvironmentError,
    > {
        unreachable!()
    }
    async fn create_volume(
        &self,
        _: &factory_core::environments::Environment,
    ) -> Result<(), factory_core::environments::EnvironmentError> {
        unreachable!()
    }
    async fn start(
        &self,
        _: &factory_core::environments::Environment,
    ) -> Result<(), factory_core::environments::EnvironmentError> {
        unreachable!()
    }
    async fn stop(
        &self,
        _: &factory_core::environments::Environment,
    ) -> Result<(), factory_core::environments::EnvironmentError> {
        unreachable!()
    }
}
struct Supervisor;
#[async_trait]
impl factory_core::environments::EnvironmentSupervisor for Supervisor {
    async fn check(&mut self) -> Result<(), factory_core::environments::EnvironmentError> {
        Ok(())
    }
}
struct RecoveryProjects {
    owner: Uuid,
    hash: CredentialHash,
    value: std::sync::Mutex<factory_core::projects::Project>,
}
impl RecoveryProjects {
    fn check(&self, a: EnvironmentActor) -> Result<(), ProjectError> {
        if a.user_id != self.owner || a.session_hash != self.hash {
            Err(ProjectError::Unauthorized)
        } else {
            Ok(())
        }
    }
}
#[async_trait]
impl factory_core::projects::ProjectStore for RecoveryProjects {
    async fn sync_repository_metadata(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: &[factory_core::projects::RepositoryMetadata],
    ) -> Result<Vec<i64>, ProjectError> {
        unreachable!()
    }
    async fn reserve(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: factory_core::projects::ImportProject,
    ) -> Result<(factory_core::projects::Project, bool), ProjectError> {
        unreachable!()
    }
    async fn get(
        &self,
        a: EnvironmentActor,
        _: Uuid,
        _: i64,
        id: Uuid,
    ) -> Result<factory_core::projects::Project, ProjectError> {
        self.check(a)?;
        let p = self.value.lock().unwrap().clone();
        if p.id != id {
            return Err(ProjectError::NotFound);
        }
        Ok(p)
    }
    async fn list(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Option<Uuid>,
    ) -> Result<factory_core::projects::ProjectPage, ProjectError> {
        unreachable!()
    }
    async fn settle(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Uuid,
        _: Uuid,
        _: bool,
    ) -> Result<factory_core::projects::Project, ProjectError> {
        unreachable!()
    }
    async fn recover_stopped(
        &self,
        a: EnvironmentActor,
        env: Uuid,
        generation: i64,
        _: chrono::DateTime<Utc>,
        id: Uuid,
    ) -> Result<factory_core::projects::Project, ProjectError> {
        self.check(a)?;
        let mut p = self.value.lock().unwrap();
        if p.id != id
            || p.environment_id != env
            || p.environment_generation >= generation
            || p.state != factory_core::projects::ProjectState::Importing
        {
            return Err(ProjectError::Conflict);
        }
        p.state = factory_core::projects::ProjectState::Failed;
        Ok(p.clone())
    }
    async fn recoverable(
        &self,
        a: EnvironmentActor,
    ) -> Result<Vec<factory_core::projects::RecoverableImport>, ProjectError> {
        self.check(a)?;
        let p = self.value.lock().unwrap();
        Ok(
            if p.state == factory_core::projects::ProjectState::Importing {
                vec![factory_core::projects::RecoverableImport {
                    project_id: p.id,
                    operation_id: p.operation_id,
                    environment_id: p.environment_id,
                    environment_generation: p.environment_generation,
                }]
            } else {
                vec![]
            },
        )
    }
    async fn confirm_recovered(
        &self,
        a: EnvironmentActor,
        env: Uuid,
        _: i64,
        id: Uuid,
        operation: Uuid,
    ) -> Result<factory_core::projects::Project, ProjectError> {
        self.check(a)?;
        let mut p = self.value.lock().unwrap();
        if p.id != id
            || p.environment_id != env
            || p.operation_id != operation
            || p.state != factory_core::projects::ProjectState::Failed
        {
            return Err(ProjectError::Conflict);
        }
        p.state = factory_core::projects::ProjectState::Ready;
        Ok(p.clone())
    }
    async fn retry(
        &self,
        _: EnvironmentActor,
        _: Uuid,
        _: i64,
        _: Uuid,
        _: Uuid,
        _: Uuid,
    ) -> Result<factory_core::projects::Project, ProjectError> {
        unreachable!()
    }
}
struct RecoveryDriver;
#[async_trait]
impl runtime::ProjectDriver for RecoveryDriver {
    async fn destination_available(
        &self,
        _: &factory_core::environments::Environment,
        _: &str,
    ) -> Result<bool, factory_adapters::projects::ProjectRuntimeError> {
        Ok(true)
    }
    async fn setup_status(
        &self,
        _: &factory_core::environments::Environment,
    ) -> Result<bool, factory_adapters::projects::ProjectRuntimeError> {
        unreachable!()
    }
    async fn public_key_status(
        &self,
        _: &factory_core::environments::Environment,
    ) -> Result<
        factory_adapters::projects::SshKeyStatus,
        factory_adapters::projects::ProjectRuntimeError,
    > {
        unreachable!()
    }
    async fn mark_key_registered(
        &self,
        _: &factory_core::environments::Environment,
        _: &str,
    ) -> Result<(), factory_adapters::projects::ProjectRuntimeError> {
        unreachable!()
    }
    async fn rotate_missing_key(
        &self,
        _: &factory_core::environments::Environment,
        _: &str,
    ) -> Result<
        factory_adapters::projects::SshKeyStatus,
        factory_adapters::projects::ProjectRuntimeError,
    > {
        unreachable!()
    }
    async fn configure_git_identity(
        &self,
        _: &factory_core::environments::Environment,
        _: &str,
        _: &str,
    ) -> Result<(), factory_adapters::projects::ProjectRuntimeError> {
        unreachable!()
    }
    async fn clone_project(
        &self,
        _: &factory_core::environments::Environment,
        _: &factory_core::projects::Project,
    ) -> Result<(), factory_adapters::projects::ProjectRuntimeError> {
        unreachable!()
    }
    async fn inspect_project(
        &self,
        _: &factory_core::environments::Environment,
        _: &factory_core::projects::Project,
    ) -> Result<
        factory_adapters::projects::ProjectDirectory,
        factory_adapters::projects::ProjectRuntimeError,
    > {
        Ok(factory_adapters::projects::ProjectDirectory::Ready)
    }
}
#[tokio::test]
async fn stopped_import_recovers_without_github_credentials_and_reuses_original_receipt() {
    use factory_application::environments::{
        EnvironmentAvailability, EnvironmentReconciler, EnvironmentService,
    };
    use factory_application::projects::ProjectService;
    use factory_core::environments::*;
    use factory_core::projects::*;
    let raw = random_token().unwrap();
    let hash = hash_token(&raw).unwrap();
    let owner = Uuid::new_v4();
    let environment = Uuid::new_v4();
    let session = ActiveSession {
        user: User {
            id: owner,
            display_name: "Fixture".into(),
            handle: "fixture".into(),
            provider: "github".into(),
            role: Role::Member,
            status: AccessStatus::Active,
            version: 1,
            access_enabled_at: Some(Utc::now()),
        },
        csrf_token: "fixture-csrf".into(),
        expires_at: Utc::now() + Duration::hours(1),
    };
    let auth = AuthState::new(
        Arc::new(IdentityService::new(
            Arc::new(Store { hash, session }),
            Arc::new(Provider),
        )),
        &RuntimeConfig::from_values(|_| None).unwrap(),
    );
    let config = EnvironmentStart {
        installation_id: Uuid::new_v4(),
        image_id: "fixture".into(),
        policy_version: "fixture".into(),
    };
    let env = Arc::new(RecoveryEnvironment {
        owner,
        hash,
        value: std::sync::Mutex::new(Environment {
            id: environment,
            user_id: owner,
            installation_id: config.installation_id,
            desired_state: DesiredState::Stopped,
            observed_state: ObservedState::Stopped,
            generation: 2,
            provisioning_state: ProvisioningState::VolumeReady,
            volume_name: "fixture".into(),
            container_name: "fixture".into(),
            image_id: config.image_id.clone(),
            policy_version: config.policy_version.clone(),
            observed_at: Some(Utc::now() + Duration::seconds(1)),
            error_code: None,
            next_reconcile_at: Utc::now(),
        }),
    });
    let availability = EnvironmentAvailability::default();
    EnvironmentReconciler::new(
        env.clone(),
        Arc::new(NoRuntime),
        Box::new(Supervisor),
        config.clone(),
        availability.clone(),
    )
    .unwrap()
    .next_batch()
    .await
    .unwrap();
    let environments =
        Arc::new(EnvironmentService::new(env.clone(), config, availability.clone()).unwrap());
    let id = Uuid::new_v4();
    let operation = Uuid::new_v4();
    let store = Arc::new(RecoveryProjects {
        owner,
        hash,
        value: std::sync::Mutex::new(Project {
            id,
            user_id: owner,
            repository_id: 7,
            full_name: "fixture/repo".into(),
            cloned_full_name: "fixture/repo".into(),
            directory: format!("project-{id}"),
            state: ProjectState::Importing,
            operation_id: operation,
            environment_id: environment,
            environment_generation: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }),
    });
    let projects = Arc::new(ProjectService::new(environments.clone(), store.clone()));
    let runtime = Arc::new(runtime::ProjectsRuntime {
        environments,
        projects,
        store: store.clone(),
        docker: Arc::new(RecoveryDriver),
        access: None,
        availability,
        installation: Arc::new(()),
        jobs: Arc::new(tokio::sync::Semaphore::new(4)),
        active: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
    });
    let app: Router = documented_router(Some(auth), Some(runtime)).into();
    let cookie = format!("factory_session={raw}");
    let get = Request::builder()
        .uri("/api/projects/recoverable")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(get).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json[0]["projectId"], id.to_string());
    let post = |path: String| {
        Request::builder()
            .method("POST")
            .uri(path)
            .header(header::COOKIE, &cookie)
            .header(header::ORIGIN, "http://localhost:3000")
            .header("x-csrf-token", "fixture-csrf")
            .body(Body::empty())
            .unwrap()
    };
    {
        let mut e = env.value.lock().unwrap();
        e.desired_state = DesiredState::Running;
        e.observed_state = ObservedState::Running;
    }
    assert_eq!(
        app.clone()
            .oneshot(post(format!("/api/projects/{id}/recover")))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    {
        let mut e = env.value.lock().unwrap();
        e.desired_state = DesiredState::Stopped;
        e.observed_state = ObservedState::Stopped;
    }
    assert_eq!(
        app.clone()
            .oneshot(post(format!("/api/projects/{id}/recover")))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    {
        let mut e = env.value.lock().unwrap();
        e.desired_state = DesiredState::Running;
        e.observed_state = ObservedState::Running;
    }
    let response = app
        .oneshot(post(format!("/api/projects/{id}/retry")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(json["state"], "ready");
    assert_eq!(store.value.lock().unwrap().operation_id, operation);
}

struct ConnectedRepositoryStore {
    connected: Arc<std::sync::atomic::AtomicBool>,
}
#[async_trait]
impl RepositoryAccessStore for ConnectedRepositoryStore {
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
        _: i64,
    ) -> Result<(), RepositoryAccessError> {
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
        if !self.connected.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(None);
        }
        Ok(Some(RepositoryCredential {
            subject: "123".into(),
            authorization_epoch: 1,
            login: "fixture".into(),
            email: "fixture@example.test".into(),
            encrypted: EncryptedRepositoryToken {
                key_id: "fixture".into(),
                nonce: vec![0; 12],
                ciphertext: vec![0; 16],
            },
        }))
    }
    async fn delete_repository_credential(
        &self,
        _: RepositoryActor,
    ) -> Result<(), RepositoryAccessError> {
        unreachable!()
    }
}
struct TestRepositoryProvider {
    result: std::sync::Mutex<Result<RepositoryOriginVerification, RepositoryAccessError>>,
    calls: std::sync::Mutex<Vec<(i64, String)>>,
}
#[async_trait]
impl RepositoryAccessProvider for TestRepositoryProvider {
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
        repository_id: i64,
        cloned_full_name: &str,
    ) -> Result<RepositoryOriginVerification, RepositoryAccessError> {
        self.calls
            .lock()
            .unwrap()
            .push((repository_id, cloned_full_name.into()));
        *self.result.lock().unwrap()
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
}
struct UnusedCipher;
impl RepositoryTokenCipher for UnusedCipher {
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
        RepositoryAccessToken::new("synthetic-test-token".into())
    }
}
struct UnusedKeys;
#[async_trait]
impl RepositoryPublicKeySource for UnusedKeys {
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
#[derive(Default)]
struct ConnectionDriver {
    ready: std::sync::atomic::AtomicBool,
    reads: std::sync::atomic::AtomicUsize,
    mutations: std::sync::atomic::AtomicUsize,
    connected: Arc<std::sync::atomic::AtomicBool>,
    disconnect_on_read: std::sync::atomic::AtomicBool,
}
#[async_trait]
impl runtime::ProjectDriver for ConnectionDriver {
    async fn destination_available(
        &self,
        _: &factory_core::environments::Environment,
        _: &str,
    ) -> Result<bool, factory_adapters::projects::ProjectRuntimeError> {
        Ok(true)
    }
    async fn setup_status(
        &self,
        _: &factory_core::environments::Environment,
    ) -> Result<bool, factory_adapters::projects::ProjectRuntimeError> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self
            .disconnect_on_read
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            self.connected
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(self.ready.load(std::sync::atomic::Ordering::SeqCst))
    }
    async fn public_key_status(
        &self,
        _: &factory_core::environments::Environment,
    ) -> Result<
        factory_adapters::projects::SshKeyStatus,
        factory_adapters::projects::ProjectRuntimeError,
    > {
        self.mutations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(factory_adapters::projects::ProjectRuntimeError::Unavailable)
    }
    async fn mark_key_registered(
        &self,
        _: &factory_core::environments::Environment,
        _: &str,
    ) -> Result<(), factory_adapters::projects::ProjectRuntimeError> {
        self.mutations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(factory_adapters::projects::ProjectRuntimeError::Unavailable)
    }
    async fn rotate_missing_key(
        &self,
        _: &factory_core::environments::Environment,
        _: &str,
    ) -> Result<
        factory_adapters::projects::SshKeyStatus,
        factory_adapters::projects::ProjectRuntimeError,
    > {
        self.mutations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(factory_adapters::projects::ProjectRuntimeError::Unavailable)
    }
    async fn configure_git_identity(
        &self,
        _: &factory_core::environments::Environment,
        _: &str,
        _: &str,
    ) -> Result<(), factory_adapters::projects::ProjectRuntimeError> {
        self.mutations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(factory_adapters::projects::ProjectRuntimeError::Unavailable)
    }
    async fn clone_project(
        &self,
        _: &factory_core::environments::Environment,
        _: &factory_core::projects::Project,
    ) -> Result<(), factory_adapters::projects::ProjectRuntimeError> {
        unreachable!()
    }
    async fn inspect_project(
        &self,
        _: &factory_core::environments::Environment,
        _: &factory_core::projects::Project,
    ) -> Result<
        factory_adapters::projects::ProjectDirectory,
        factory_adapters::projects::ProjectRuntimeError,
    > {
        unreachable!()
    }
}

#[tokio::test]
async fn project_verification_and_connection_are_read_only_and_report_provider_errors() {
    use factory_application::{
        environments::{EnvironmentAvailability, EnvironmentReconciler, EnvironmentService},
        identity::RepositoryAccessService,
        projects::ProjectService,
    };
    use factory_core::{environments::*, projects::*};
    use std::sync::atomic::Ordering;
    let raw = random_token().unwrap();
    let hash = hash_token(&raw).unwrap();
    let owner = Uuid::new_v4();
    let environment = Uuid::new_v4();
    let session = ActiveSession {
        user: User {
            id: owner,
            display_name: "Fixture".into(),
            handle: "fixture".into(),
            provider: "github".into(),
            role: Role::Member,
            status: AccessStatus::Active,
            version: 1,
            access_enabled_at: Some(Utc::now()),
        },
        csrf_token: "fixture-csrf".into(),
        expires_at: Utc::now() + Duration::hours(1),
    };
    let mut other_session = session.clone();
    other_session.user.id = Uuid::new_v4();
    let auth = AuthState::new(
        Arc::new(IdentityService::new(
            Arc::new(Store { hash, session }),
            Arc::new(Provider),
        )),
        &RuntimeConfig::from_values(|_| None).unwrap(),
    );
    let config = EnvironmentStart {
        installation_id: Uuid::new_v4(),
        image_id: "fixture".into(),
        policy_version: "fixture".into(),
    };
    let env = Arc::new(RecoveryEnvironment {
        owner,
        hash,
        value: std::sync::Mutex::new(Environment {
            id: environment,
            user_id: owner,
            installation_id: config.installation_id,
            desired_state: DesiredState::Running,
            observed_state: ObservedState::Running,
            generation: 1,
            provisioning_state: ProvisioningState::VolumeReady,
            volume_name: "fixture".into(),
            container_name: "fixture".into(),
            image_id: config.image_id.clone(),
            policy_version: config.policy_version.clone(),
            observed_at: Some(Utc::now() + Duration::seconds(1)),
            error_code: None,
            next_reconcile_at: Utc::now(),
        }),
    });
    let availability = EnvironmentAvailability::default();
    EnvironmentReconciler::new(
        env.clone(),
        Arc::new(NoRuntime),
        Box::new(Supervisor),
        config.clone(),
        availability.clone(),
    )
    .unwrap()
    .next_batch()
    .await
    .unwrap();
    let environments =
        Arc::new(EnvironmentService::new(env, config, availability.clone()).unwrap());
    let id = Uuid::new_v4();
    let store = Arc::new(RecoveryProjects {
        owner,
        hash,
        value: std::sync::Mutex::new(Project {
            id,
            user_id: owner,
            repository_id: 7,
            full_name: "renamed/repo".into(),
            cloned_full_name: "fixture/repo".into(),
            directory: format!("project-{id}"),
            state: ProjectState::Ready,
            operation_id: Uuid::new_v4(),
            environment_id: environment,
            environment_generation: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }),
    });
    let projects = Arc::new(ProjectService::new(environments.clone(), store.clone()));
    let driver = Arc::new(ConnectionDriver::default());
    driver.connected.store(true, Ordering::SeqCst);
    let provider = Arc::new(TestRepositoryProvider {
        result: std::sync::Mutex::new(Ok(RepositoryOriginVerification::Matched)),
        calls: std::sync::Mutex::new(Vec::new()),
    });
    let access = Arc::new(RepositoryAccessService::new(
        Arc::new(ConnectedRepositoryStore {
            connected: driver.connected.clone(),
        }),
        provider.clone(),
        Arc::new(UnusedCipher),
        Arc::new(UnusedKeys),
    ));
    let runtime = Arc::new(runtime::ProjectsRuntime {
        environments,
        projects,
        store,
        docker: driver.clone(),
        access: Some(access),
        availability,
        installation: Arc::new(()),
        jobs: Arc::new(tokio::sync::Semaphore::new(4)),
        active: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
    });
    let other_auth = AuthState::new(
        Arc::new(IdentityService::new(
            Arc::new(Store {
                hash,
                session: other_session,
            }),
            Arc::new(Provider),
        )),
        &RuntimeConfig::from_values(|_| None).unwrap(),
    );
    let other_app: Router = documented_router(Some(other_auth), Some(runtime.clone())).into();
    let app: Router = documented_router(Some(auth), Some(runtime)).into();
    let cookie = format!("factory_session={raw}");
    let verification = |id: Uuid, cookie: Option<&str>| {
        let mut req = Request::builder().uri(format!("/api/projects/{id}/verification"));
        if let Some(cookie) = cookie {
            req = req.header(header::COOKIE, cookie);
        }
        req.body(Body::empty()).unwrap()
    };
    assert_eq!(
        app.clone()
            .oneshot(verification(id, None))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.clone()
            .oneshot(verification(Uuid::new_v4(), Some(&cookie)))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        other_app
            .oneshot(verification(id, Some(&cookie)))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(provider.calls.lock().unwrap().is_empty());
    for (result, expected_status, expected_body) in [
        (
            Ok(RepositoryOriginVerification::Matched),
            StatusCode::OK,
            "matched",
        ),
        (
            Ok(RepositoryOriginVerification::Changed),
            StatusCode::OK,
            "changed",
        ),
        (
            Err(RepositoryAccessError::ProviderUnauthorized),
            StatusCode::CONFLICT,
            "provider_unauthorized",
        ),
        (
            Err(RepositoryAccessError::RateLimited),
            StatusCode::TOO_MANY_REQUESTS,
            "provider_rate_limited",
        ),
        (
            Err(RepositoryAccessError::Unavailable),
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
        ),
    ] {
        *provider.result.lock().unwrap() = result;
        let response = app
            .clone()
            .oneshot(verification(id, Some(&cookie)))
            .await
            .unwrap();
        assert_eq!(response.status(), expected_status);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json[if expected_status == StatusCode::OK {
                "status"
            } else {
                "code"
            }],
            expected_body
        );
    }
    {
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 5);
        assert!(calls.iter().all(|call| call == &(7, "fixture/repo".into())));
    }
    for (ready, expected) in [(false, "disconnected"), (true, "connected")] {
        driver.ready.store(ready, Ordering::SeqCst);
        let request = Request::builder()
            .uri("/api/projects/connection")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], expected);
        if !ready {
            assert!(json["login"].is_null());
            assert!(json["email"].is_null());
        }
    }
    driver.disconnect_on_read.store(true, Ordering::SeqCst);
    let request = Request::builder()
        .uri("/api/projects/connection")
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::CONFLICT
    );
    assert_eq!(driver.reads.load(Ordering::SeqCst), 3);
    assert_eq!(
        driver.mutations.load(Ordering::SeqCst),
        0,
        "GET must never generate a key or configure Git"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/api/projects/repositories")
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, "http://localhost:3000")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-csrf-token", "fixture-csrf")
        .body(Body::from(r#"{"page":1}"#))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], "provider_unauthorized");
}
