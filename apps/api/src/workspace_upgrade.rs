use crate::config::RuntimeConfig;
use factory_adapters::{
    egress::EgressMetadata,
    environments::{DockerEnvironmentRuntime, InstallationGuard, PreparationMetadata},
};
use factory_core::environments::{EnvironmentError, EnvironmentStart, EnvironmentSupervisor};
use factory_database::identity::IdentityDatabase;
use std::{fs::OpenOptions, path::Path, sync::Arc};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Check,
    Apply,
    Rollback,
}

struct Prepared {
    workspace: PreparationMetadata,
    proxy: EgressMetadata,
}

impl Prepared {
    fn load(directory: &Path) -> Result<Self, EnvironmentError> {
        let workspace = PreparationMetadata::load(directory)?;
        let proxy = EgressMetadata::load(directory)?;
        if workspace.installation_id() != proxy.installation_id {
            return Err(EnvironmentError::OwnershipConflict);
        }
        Ok(Self { workspace, proxy })
    }

    fn runtime(&self, can_mutate: bool) -> Result<DockerEnvironmentRuntime, EnvironmentError> {
        DockerEnvironmentRuntime::connect(
            self.workspace.clone(),
            self.proxy.clone(),
            Arc::new(move || can_mutate),
        )
    }
}

pub async fn run(
    config: &RuntimeConfig,
    database: &IdentityDatabase,
    action: Action,
) -> Result<(), &'static str> {
    let directory = config.workspace().join(".factory/environments");
    let journal = directory.join("upgrade");
    let previous = Prepared::load(&journal.join("previous")).map_err(EnvironmentError::code)?;
    let candidate = Prepared::load(&journal.join("candidate")).map_err(EnvironmentError::code)?;
    let installation = previous.workspace.installation_id();
    if candidate.workspace.installation_id() != installation
        || candidate.workspace.policy_version() != previous.workspace.policy_version()
    {
        return Err("WORKSPACE_UPGRADE_INCOMPATIBLE_POLICY");
    }
    let (source, target) = if action == Action::Rollback {
        (&candidate, &previous)
    } else {
        (&previous, &candidate)
    };
    if action == Action::Rollback
        && source
            .proxy
            .host_ipv4
            .iter()
            .any(|ip| !target.proxy.host_ipv4.contains(ip))
    {
        // Restoring the old deny list would allow newly present host addresses.
        return Err("WORKSPACE_UPGRADE_STALE_ROLLBACK_DENY_LIST");
    }
    let replace_proxy = source.proxy.image_id != target.proxy.image_id
        || source.proxy.host_ipv4 != target.proxy.host_ipv4;
    let replace_image = source.workspace.image_id() != target.workspace.image_id();
    let can_mutate = action != Action::Check && (replace_image || replace_proxy);
    let mut supervisor = None;
    let _installation_guard = if can_mutate {
        let guard = InstallationGuard::acquire(&directory).map_err(EnvironmentError::code)?;
        supervisor = Some(
            database
                .environment_supervisor(installation)
                .await
                .map_err(EnvironmentError::code)?,
        );
        Some(guard)
    } else {
        None
    };
    let source_runtime = source.runtime(can_mutate).map_err(EnvironmentError::code)?;
    let target_runtime = target.runtime(can_mutate).map_err(EnvironmentError::code)?;
    let source_proxy = source_runtime.egress();
    match source_proxy
        .upgrade_proxy()
        .await
        .map_err(EnvironmentError::code)?
    {
        None | Some(true) => {}
        Some(false)
            if replace_proxy
                && target_runtime
                    .egress()
                    .upgrade_proxy()
                    .await
                    .map_err(EnvironmentError::code)?
                    == Some(true) => {}
        Some(false) => return Err("WORKSPACE_UPGRADE_UNKNOWN_PROXY"),
    }
    if !replace_image && !replace_proxy {
        return Ok(());
    }
    let store = database.environment_store();
    if store
        .upgrade_has_active_work(installation)
        .await
        .map_err(EnvironmentError::code)?
    {
        return Err("WORKSPACE_UPGRADE_ACTIVE_WORK");
    }
    let environments = store
        .upgrade_inventory(installation)
        .await
        .map_err(EnvironmentError::code)?;
    for environment in &environments {
        if environment.image_id != source.workspace.image_id()
            && environment.image_id != target.workspace.image_id()
        {
            return Err("WORKSPACE_UPGRADE_UNKNOWN_IMAGE");
        }
        source_runtime
            .verify_upgrade_workspace(environment)
            .await
            .map_err(EnvironmentError::code)?;
    }
    let Some(mut supervisor) = supervisor else {
        return Ok(());
    };
    // Once this file exists, restoring the previous release also requires a
    // verified reverse transition. Create it before the first Docker mutation.
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(journal.join("mutating"))
        .and_then(|file| file.sync_all())
        .map_err(|_| "WORKSPACE_UPGRADE_JOURNAL_FAILED")?;
    for environment in &environments {
        supervisor.check().await.map_err(EnvironmentError::code)?;
        let replace_container = environment.image_id != target.workspace.image_id();
        source_runtime
            .stop_for_upgrade(environment, replace_container)
            .await
            .map_err(EnvironmentError::code)?;
    }
    if replace_proxy
        && source_proxy
            .upgrade_proxy()
            .await
            .map_err(EnvironmentError::code)?
            == Some(true)
    {
        supervisor.check().await.map_err(EnvironmentError::code)?;
        source_proxy
            .remove_upgrade_proxy()
            .await
            .map_err(EnvironmentError::code)?;
    }
    supervisor.check().await.map_err(EnvironmentError::code)?;
    store
        .complete_runtime_upgrade(
            EnvironmentStart {
                installation_id: installation,
                image_id: source.workspace.image_id().to_owned(),
                policy_version: source.workspace.policy_version().to_owned(),
            },
            target.workspace.image_id().to_owned(),
            environments.len(),
        )
        .await
        .map_err(EnvironmentError::code)
}
