use super::DockerProjectRuntime;
use async_trait::async_trait;
use factory_core::{
    environments::Environment,
    features::{
        FeatureDeliveryRuntime, FeatureRemoteRefs, GitSnapshot, WorkspaceDiff, WorkspaceError,
    },
    projects::Project,
};
use uuid::Uuid;

#[async_trait]
impl FeatureDeliveryRuntime for DockerProjectRuntime {
    async fn remote(
        &self,
        env: &Environment,
        project: &Project,
        id: Uuid,
    ) -> Result<FeatureRemoteRefs, WorkspaceError> {
        let remote = self.feature_remote(env, project, id).await?;
        Ok(FeatureRemoteRefs {
            base_oid: remote.base_oid,
            head_oid: remote.head_oid,
            managed_head_oid: remote.managed_head_oid,
        })
    }

    async fn push_reviewed(
        &self,
        env: &Environment,
        project: &Project,
        id: Uuid,
        snapshot: &GitSnapshot,
    ) -> Result<(), WorkspaceError> {
        self.push_feature(env, project, id, snapshot).await
    }

    async fn stage_refresh(
        &self,
        env: &Environment,
        project: &Project,
        id: Uuid,
        candidate: Uuid,
        expected: &GitSnapshot,
    ) -> Result<GitSnapshot, WorkspaceError> {
        self.stage_feature_refresh(env, project, id, candidate, expected)
            .await
    }

    async fn confirm_refresh(
        &self,
        env: &Environment,
        project: &Project,
        id: Uuid,
        candidate: Uuid,
        expected: &GitSnapshot,
        proposed: &GitSnapshot,
    ) -> Result<WorkspaceDiff, WorkspaceError> {
        self.confirm_feature_refresh(env, project, id, candidate, expected, proposed)
            .await
    }
}
