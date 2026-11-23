use super::{CLONE_TIMEOUT, DockerProjectRuntime, ProjectRuntimeError, validate_project};
use async_trait::async_trait;
use factory_core::{
    environments::Environment,
    features::{
        FeatureBranch, FeatureWorkspaceRuntime, GitSnapshot, MergeResolutionCandidate, TaskAttempt,
        TaskWorkspace, WorkspaceAccess, WorkspaceDiff, WorkspaceError,
    },
    projects::{Project, ProjectState},
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::fmt::Write;
use uuid::Uuid;

// The feature controller has separate request/response limits from project imports.
const REQUEST_LIMIT: usize = 192 * 1024;
const RESPONSE_LIMIT: usize = 1024 * 1024;
const RESOLUTION_SCOPE_LIMIT: usize = 64 * 128;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeatureRemote {
    pub base_oid: Option<String>,
    pub head_oid: Option<String>,
    pub managed_head_oid: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BranchReply {
    base_ref: String,
    base_oid: String,
    head_ref: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotReply {
    git: SnapshotIdentity,
    patch: String,
    changed_paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotIdentity {
    base_oid: String,
    head_oid: String,
    patch_digest: String,
}

#[async_trait]
impl FeatureWorkspaceRuntime for DockerProjectRuntime {
    async fn prepare_initialization(
        &self,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
        candidate_id: Uuid,
    ) -> Result<FeatureBranch, WorkspaceError> {
        let mut request =
            feature_request(env, project, feature_id, "feature_stage_initialization")?;
        if candidate_id.is_nil() {
            return Err(WorkspaceError::Invalid);
        }
        request["candidate"] = json!(candidate_id);
        parse_branch(self.run_feature(env, request).await?, feature_id)
    }

    async fn confirm_initialization(
        &self,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
        candidate_id: Uuid,
        expected: &FeatureBranch,
    ) -> Result<FeatureBranch, WorkspaceError> {
        let mut request =
            feature_request(env, project, feature_id, "feature_confirm_initialization")?;
        if candidate_id.is_nil() {
            return Err(WorkspaceError::Invalid);
        }
        request["candidate"] = json!(candidate_id);
        request["expected_branch"] = json!({
            "base_ref": expected.base_ref,
            "base_oid": expected.base_oid,
            "head_ref": expected.head_ref,
        });
        let branch = parse_branch(self.run_feature(env, request).await?, feature_id)?;
        if &branch != expected {
            return Err(WorkspaceError::Conflict);
        }
        Ok(branch)
    }

    async fn prepare_task(
        &self,
        env: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
    ) -> Result<(), WorkspaceError> {
        let request = attempt_request(env, project, workspace, "feature_prepare")?;
        expect_reply(self.run_feature(env, request).await?, "ready")
    }

    async fn checkpoint(
        &self,
        env: &Environment,
        project: &Project,
        attempt: &TaskAttempt,
    ) -> Result<String, WorkspaceError> {
        let mut request =
            attempt_request(env, project, &attempt.claim.workspace, "feature_checkpoint")?;
        request["scope"] = json!(attempt.claim.task.paths);
        parse_oid(self.run_feature(env, request).await?)
    }

    async fn integrate(
        &self,
        env: &Environment,
        project: &Project,
        attempt: &TaskAttempt,
        expected_head: &str,
    ) -> Result<String, WorkspaceError> {
        if !valid_oid(expected_head) {
            return Err(WorkspaceError::Invalid);
        }
        let mut request =
            attempt_request(env, project, &attempt.claim.workspace, "feature_integrate")?;
        request["scope"] = json!(attempt.claim.task.paths);
        request["expected_head"] = json!(expected_head);
        parse_oid(self.run_feature(env, request).await?)
    }

    async fn snapshot(
        &self,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
    ) -> Result<WorkspaceDiff, WorkspaceError> {
        let request = feature_request(env, project, feature_id, "feature_snapshot")?;
        parse_snapshot(self.run_feature(env, request).await?)
    }
}

impl DockerProjectRuntime {
    /// Inspect exactly the two refs of the app-owned feature on its canonical remote.
    pub async fn feature_remote(
        &self,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
    ) -> Result<FeatureRemote, WorkspaceError> {
        let request = feature_request(env, project, feature_id, "feature_remote")?;
        let reply: FeatureRemote = self.run_feature(env, request).await?;
        if !reply.base_oid.as_deref().is_none_or(valid_oid)
            || !reply.head_oid.as_deref().is_none_or(valid_oid)
            || reply
                .managed_head_oid
                .as_ref()
                .is_some_and(|oid| reply.head_oid.as_ref() != Some(oid))
        {
            return Err(WorkspaceError::Uncertain);
        }
        Ok(reply)
    }

    /// Use only after an authenticated request has durably claimed the reviewed snapshot.
    pub async fn push_feature(
        &self,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
        snapshot: &GitSnapshot,
    ) -> Result<(), WorkspaceError> {
        let request = snapshot_request(env, project, feature_id, "feature_push", snapshot)?;
        expect_reply(self.run_feature(env, request).await?, "pushed")
    }

    pub(super) async fn stage_feature_refresh(
        &self,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
        candidate: Uuid,
        expected: &GitSnapshot,
    ) -> Result<GitSnapshot, WorkspaceError> {
        let mut request =
            snapshot_request(env, project, feature_id, "feature_stage_refresh", expected)?;
        if candidate.is_nil() {
            return Err(WorkspaceError::Invalid);
        }
        request["candidate"] = json!(candidate);
        parse_snapshot_identity(self.run_feature(env, request).await?)
    }

    pub(super) async fn confirm_feature_refresh(
        &self,
        env: &Environment,
        project: &Project,
        feature_id: Uuid,
        candidate: Uuid,
        expected: &GitSnapshot,
        proposed: &GitSnapshot,
    ) -> Result<WorkspaceDiff, WorkspaceError> {
        let mut request = snapshot_request(
            env,
            project,
            feature_id,
            "feature_confirm_refresh",
            expected,
        )?;
        if candidate.is_nil() || !valid_snapshot_identity(proposed) {
            return Err(WorkspaceError::Invalid);
        }
        request["candidate"] = json!(candidate);
        request["proposed_snapshot"] = json!({
            "base_oid": proposed.base_oid,
            "head_oid": proposed.head_oid,
            "patch_digest": proposed.patch_digest,
        });
        let confirmed = parse_snapshot(self.run_feature(env, request).await?)?;
        if &confirmed.git != proposed {
            return Err(WorkspaceError::Uncertain);
        }
        Ok(confirmed)
    }

    pub(super) async fn stage_feature_merge_resolution(
        &self,
        env: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
        reviewed: &GitSnapshot,
        approved_paths: &[String],
    ) -> Result<MergeResolutionCandidate, WorkspaceError> {
        let mut request = resolution_request(
            env,
            project,
            workspace,
            "feature_stage_resolution",
            reviewed,
        )?;
        if !(1..=RESOLUTION_SCOPE_LIMIT).contains(&approved_paths.len()) {
            return Err(WorkspaceError::Invalid);
        }
        request["scope"] = json!(approved_paths);
        let reply: ResolutionReply = self
            .run_feature(env, request)
            .await
            .map_err(resolution_error)?;
        parse_resolution_candidate(reply, reviewed)
    }

    pub(super) async fn discard_feature_merge_resolution(
        &self,
        env: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
        reviewed: &GitSnapshot,
    ) -> Result<(), WorkspaceError> {
        let request = resolution_request(
            env,
            project,
            workspace,
            "feature_discard_resolution",
            reviewed,
        )?;
        let reply: DiscardResolutionReply = self
            .run_feature(env, request)
            .await
            .map_err(resolution_error)?;
        if !reply.discarded {
            return Err(WorkspaceError::Uncertain);
        }
        Ok(())
    }

    pub(super) async fn adopt_feature_merge_resolution(
        &self,
        env: &Environment,
        project: &Project,
        workspace: &TaskWorkspace,
        reviewed: &GitSnapshot,
        candidate: &MergeResolutionCandidate,
    ) -> Result<WorkspaceDiff, WorkspaceError> {
        let mut request = resolution_request(
            env,
            project,
            workspace,
            "feature_adopt_resolution",
            reviewed,
        )?;
        if !valid_resolution_candidate(candidate, reviewed) {
            return Err(WorkspaceError::Invalid);
        }
        request["target_oid"] = json!(candidate.target_oid);
        request["paths"] = json!(candidate.paths);
        let diff = parse_snapshot(
            self.run_feature(env, request)
                .await
                .map_err(resolution_error)?,
        )?;
        if diff.git.base_oid != candidate.target_oid || diff.git.head_oid == reviewed.head_oid {
            return Err(WorkspaceError::Uncertain);
        }
        Ok(diff)
    }

    async fn run_feature<T: DeserializeOwned>(
        &self,
        env: &Environment,
        request: Value,
    ) -> Result<T, WorkspaceError> {
        let response = self
            .run_bounded(env, request, CLONE_TIMEOUT, REQUEST_LIMIT, RESPONSE_LIMIT)
            .await?;
        serde_json::from_value(response).map_err(|_| WorkspaceError::Uncertain)
    }
}

fn feature_request(
    env: &Environment,
    project: &Project,
    feature_id: Uuid,
    op: &str,
) -> Result<Value, WorkspaceError> {
    validate_project(env, project)?;
    if feature_id.is_nil() || project.state != ProjectState::Ready {
        return Err(WorkspaceError::Invalid);
    }
    Ok(json!({
        "op": op,
        "project": project.id,
        "operation": project.operation_id,
        "directory": project.directory,
        "repository": project.cloned_full_name,
        "repository_id": project.repository_id,
        "feature": feature_id,
    }))
}

fn attempt_request(
    env: &Environment,
    project: &Project,
    workspace: &TaskWorkspace,
    op: &str,
) -> Result<Value, WorkspaceError> {
    if workspace.attempt_id.is_nil() || !valid_oid(&workspace.base_oid) {
        return Err(WorkspaceError::Invalid);
    }
    let mut request = feature_request(env, project, workspace.feature_id, op)?;
    request["attempt"] = json!(workspace.attempt_id);
    request["base_oid"] = json!(workspace.base_oid);
    request["writable"] = json!(workspace.access == WorkspaceAccess::Write);
    Ok(request)
}

fn snapshot_request(
    env: &Environment,
    project: &Project,
    feature_id: Uuid,
    op: &str,
    snapshot: &GitSnapshot,
) -> Result<Value, WorkspaceError> {
    if !valid_snapshot_identity(snapshot) {
        return Err(WorkspaceError::Invalid);
    }
    let mut request = feature_request(env, project, feature_id, op)?;
    request["base_oid"] = json!(snapshot.base_oid);
    request["head_oid"] = json!(snapshot.head_oid);
    request["patch_digest"] = json!(snapshot.patch_digest);
    Ok(request)
}

fn resolution_request(
    env: &Environment,
    project: &Project,
    workspace: &TaskWorkspace,
    op: &str,
    reviewed: &GitSnapshot,
) -> Result<Value, WorkspaceError> {
    if workspace.access != WorkspaceAccess::Write
        || workspace.attempt_id.is_nil()
        || workspace.base_oid != reviewed.head_oid
    {
        return Err(WorkspaceError::Invalid);
    }
    let mut request = snapshot_request(env, project, workspace.feature_id, op, reviewed)?;
    request["attempt"] = json!(workspace.attempt_id);
    Ok(request)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiscardResolutionReply {
    discarded: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolutionReply {
    target_oid: String,
    paths: Vec<String>,
}

fn resolution_error(error: WorkspaceError) -> WorkspaceError {
    // Older controller images reject the new operations as unknown.
    // The adapter checks the request identity before making the call.
    match error {
        WorkspaceError::Invalid => WorkspaceError::Unavailable,
        other => other,
    }
}

fn parse_resolution_candidate(
    reply: ResolutionReply,
    reviewed: &GitSnapshot,
) -> Result<MergeResolutionCandidate, WorkspaceError> {
    let candidate = MergeResolutionCandidate {
        target_oid: reply.target_oid,
        paths: reply.paths,
    };
    if !valid_resolution_candidate(&candidate, reviewed) {
        return Err(WorkspaceError::Uncertain);
    }
    Ok(candidate)
}

fn valid_resolution_candidate(
    candidate: &MergeResolutionCandidate,
    reviewed: &GitSnapshot,
) -> bool {
    valid_oid(&candidate.target_oid)
        && candidate.target_oid != reviewed.base_oid
        && (1..=128).contains(&candidate.paths.len())
        && candidate.paths.windows(2).all(|pair| pair[0] < pair[1])
        && candidate.paths.iter().all(|path| {
            !path.trim().is_empty()
                && path.len() <= 1024
                && !path.contains(['\\', ':'])
                && !path.chars().any(char::is_control)
                && path.split('/').all(|part| {
                    !matches!(part, "" | "." | "..") && !part.eq_ignore_ascii_case(".git")
                })
        })
}

fn parse_snapshot(reply: SnapshotReply) -> Result<WorkspaceDiff, WorkspaceError> {
    let git = parse_snapshot_identity(reply.git)?;
    if git.patch_digest != digest(&reply.patch) {
        return Err(WorkspaceError::Uncertain);
    }
    Ok(WorkspaceDiff {
        git,
        patch: reply.patch,
        changed_paths: reply.changed_paths,
    })
}

fn parse_snapshot_identity(reply: SnapshotIdentity) -> Result<GitSnapshot, WorkspaceError> {
    let snapshot = GitSnapshot {
        base_oid: reply.base_oid,
        head_oid: reply.head_oid,
        patch_digest: reply.patch_digest,
    };
    if !valid_snapshot_identity(&snapshot) {
        return Err(WorkspaceError::Uncertain);
    }
    Ok(snapshot)
}

fn valid_snapshot_identity(snapshot: &GitSnapshot) -> bool {
    valid_oid(&snapshot.base_oid)
        && valid_oid(&snapshot.head_oid)
        && valid_hex(&snapshot.patch_digest, 64)
}

fn parse_branch(reply: BranchReply, feature_id: Uuid) -> Result<FeatureBranch, WorkspaceError> {
    if !valid_oid(&reply.base_oid)
        || !valid_branch(&reply.base_ref)
        || reply.head_ref != format!("refs/heads/factory/features/{feature_id}")
    {
        return Err(WorkspaceError::Uncertain);
    }
    Ok(FeatureBranch {
        base_ref: reply.base_ref,
        base_oid: reply.base_oid,
        head_ref: reply.head_ref,
    })
}

fn expect_reply(reply: String, expected: &str) -> Result<(), WorkspaceError> {
    if reply == expected {
        Ok(())
    } else {
        Err(WorkspaceError::Uncertain)
    }
}

fn parse_oid(reply: String) -> Result<String, WorkspaceError> {
    if valid_oid(&reply) {
        Ok(reply)
    } else {
        Err(WorkspaceError::Uncertain)
    }
}

fn valid_branch(value: &str) -> bool {
    value.starts_with("refs/heads/")
        && value.len() > "refs/heads/".len()
        && value.len() <= 1024
        && !value.chars().any(char::is_control)
}

fn valid_oid(value: &str) -> bool {
    valid_hex(value, 40) || valid_hex(value, 64)
}

fn valid_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest(patch: &str) -> String {
    let hash = ring::digest::digest(&ring::digest::SHA256, patch.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in hash.as_ref() {
        write!(hex, "{byte:02x}").expect("writing to a String succeeds");
    }
    hex
}

impl From<ProjectRuntimeError> for WorkspaceError {
    fn from(error: ProjectRuntimeError) -> Self {
        match error {
            ProjectRuntimeError::Invalid => Self::Invalid,
            ProjectRuntimeError::Conflict => Self::Conflict,
            ProjectRuntimeError::MergeConflict => Self::MergeConflict,
            ProjectRuntimeError::ScopeViolation => Self::ScopeViolation,
            ProjectRuntimeError::DiffUnrenderable => Self::DiffUnrenderable,
            ProjectRuntimeError::Unavailable => Self::Unavailable,
            ProjectRuntimeError::Uncertain => Self::Uncertain,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_resolution_reply_binds_a_new_target_and_canonical_paths() {
        let snapshot = GitSnapshot {
            base_oid: "a".repeat(40),
            head_oid: "b".repeat(40),
            patch_digest: "c".repeat(64),
        };
        let target = "d".repeat(40);
        let reply = serde_json::from_value(json!({
            "target_oid": target,
            "paths": ["src/main.rs", "test file.txt"],
        }))
        .unwrap();
        let candidate = parse_resolution_candidate(reply, &snapshot).unwrap();
        assert_eq!(candidate.target_oid, target);
        assert_eq!(candidate.paths, ["src/main.rs", "test file.txt"]);

        for paths in [
            vec!["src/../../outside"],
            vec![".GiT/config"],
            vec!["/absolute"],
            vec!["test file.txt", "src/main.rs"],
            vec!["src/main.rs", "src/main.rs"],
        ] {
            assert_eq!(
                parse_resolution_candidate(
                    ResolutionReply {
                        target_oid: target.clone(),
                        paths: paths.into_iter().map(str::to_owned).collect(),
                    },
                    &snapshot
                ),
                Err(WorkspaceError::Uncertain),
            );
        }
        assert_eq!(
            parse_resolution_candidate(
                ResolutionReply {
                    target_oid: snapshot.base_oid.clone(),
                    paths: vec!["src/main.rs".into()],
                },
                &snapshot
            ),
            Err(WorkspaceError::Uncertain),
        );
    }
}
