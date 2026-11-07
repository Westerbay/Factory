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

    async fn refresh_base(
        &self,
        env: &Environment,
        project: &Project,
        id: Uuid,
        expected: &GitSnapshot,
    ) -> Result<WorkspaceDiff, WorkspaceError> {
        self.refresh_feature_base(env, project, id, expected).await
    }
}
