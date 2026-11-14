//! PostgreSQL project receipts, fenced by owner, session and environment generation.
use crate::connection::with_connection;
use chrono::{DateTime, Utc};
use factory_core::{environments::EnvironmentActor, projects::*};
use sqlx::{Connection, PgPool, Postgres, Row, Transaction, postgres::PgRow};
use uuid::Uuid;

const PAGE_SIZE: i64 = 100;
const COLUMNS: &str = "id,user_id,repository_id,full_name,cloned_full_name,directory,state,operation_id,environment_id,environment_generation,created_at,updated_at";

#[derive(Clone)]
pub struct PgProjectStore {
    pool: PgPool,
}
impl PgProjectStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
fn unavailable(_: impl std::fmt::Display) -> ProjectError {
    ProjectError::Unavailable
}
fn project(row: &PgRow) -> Result<Project, ProjectError> {
    let state = match row
        .try_get::<String, _>("state")
        .map_err(unavailable)?
        .as_str()
    {
        "importing" => ProjectState::Importing,
        "ready" => ProjectState::Ready,
        "failed" => ProjectState::Failed,
        "deleting" => ProjectState::Deleting,
        _ => return Err(ProjectError::Unavailable),
    };
    Ok(Project {
        id: row.try_get("id").map_err(unavailable)?,
        user_id: row.try_get("user_id").map_err(unavailable)?,
        repository_id: row.try_get("repository_id").map_err(unavailable)?,
        full_name: row.try_get("full_name").map_err(unavailable)?,
        cloned_full_name: row.try_get("cloned_full_name").map_err(unavailable)?,
        directory: row.try_get("directory").map_err(unavailable)?,
        state,
        operation_id: row.try_get("operation_id").map_err(unavailable)?,
        environment_id: row.try_get("environment_id").map_err(unavailable)?,
        environment_generation: row.try_get("environment_generation").map_err(unavailable)?,
        created_at: row.try_get("created_at").map_err(unavailable)?,
        updated_at: row.try_get("updated_at").map_err(unavailable)?,
    })
}
async fn authorize_actor(
    tx: &mut Transaction<'_, Postgres>,
    actor: EnvironmentActor,
) -> Result<(), ProjectError> {
    if actor.user_id.is_nil() {
        return Err(ProjectError::Invalid);
    }
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM factory_identity.users WHERE id=$1 FOR UPDATE")
            .bind(actor.user_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(unavailable)?;
    if status.as_deref() != Some("active") {
        return Err(ProjectError::Unauthorized);
    }
    // Keep a session-row lock until commit. Logout deletes this row without taking
    // the user lock, and must serialize before or after the complete operation.
    let session: Option<i32> = sqlx::query_scalar("SELECT 1 FROM factory_identity.sessions WHERE user_id=$1 AND token_hash=$2 AND expires_at>clock_timestamp() FOR SHARE")
        .bind(actor.user_id).bind(actor.session_hash.0.as_slice()).fetch_optional(&mut **tx).await.map_err(unavailable)?;
    if session.is_none() {
        return Err(ProjectError::Unauthorized);
    }
    Ok(())
}
async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    actor: EnvironmentActor,
    environment_id: Uuid,
    generation: i64,
) -> Result<(), ProjectError> {
    if environment_id.is_nil() || generation <= 0 {
        return Err(ProjectError::Invalid);
    }
    authorize_actor(tx, actor).await?;
    let env: Option<Uuid> = sqlx::query_scalar("SELECT id FROM factory_identity.environments WHERE id=$1 AND user_id=$2 AND generation=$3 AND desired_state='running' AND observed_state='running' AND error_code IS NULL FOR UPDATE")
        .bind(environment_id).bind(actor.user_id).bind(generation).fetch_optional(&mut **tx).await.map_err(unavailable)?;
    if env.is_none() {
        return Err(ProjectError::Conflict);
    }
    Ok(())
}
async fn has_blocking_work(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    project_id: Uuid,
) -> Result<bool, ProjectError> {
    sqlx::query_scalar(
        r#"SELECT EXISTS (
            SELECT 1 FROM factory_identity.conversation_runs r
            JOIN factory_identity.conversations c ON c.id=r.conversation_id
            WHERE c.project_id=$1 AND c.user_id=$2 AND r.status IN ('active','uncertain')
         ) OR EXISTS (
            SELECT 1 FROM factory_identity.features f
            JOIN factory_identity.feature_attempts a ON a.feature_id=f.id
            WHERE f.project_id=$1 AND f.user_id=$2 AND a.reserved
         ) OR EXISTS (
            SELECT 1 FROM factory_identity.features f
            JOIN factory_identity.feature_deliveries d ON d.feature_id=f.id
            WHERE f.project_id=$1 AND f.user_id=$2 AND d.unsettled
         ) OR EXISTS (
            SELECT 1 FROM factory_identity.features f
            JOIN factory_identity.feature_reviews r ON r.feature_id=f.id
            WHERE f.project_id=$1 AND f.user_id=$2
              AND jsonb_path_exists(r.record, '$.integrated_checks[*] ? (@.status == "Preparing" || @.status == "Launching")')
         )"#,
    )
    .bind(project_id)
    .bind(user_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(unavailable)
}

#[async_trait::async_trait]
impl ProjectStore for PgProjectStore {
    async fn begin_delete(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        id: Uuid,
    ) -> Result<Project, ProjectError> {
        if id.is_nil() {
            return Err(ProjectError::Invalid);
        }
        with_connection(&self.pool, || ProjectError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, actor, env, generation).await?;
            let row = sqlx::query(&format!("SELECT {COLUMNS} FROM factory_identity.projects WHERE id=$1 AND user_id=$2 AND environment_id=$3 FOR UPDATE"))
                .bind(id).bind(actor.user_id).bind(env).fetch_optional(&mut *tx).await.map_err(unavailable)?.ok_or(ProjectError::NotFound)?;
            let existing = project(&row)?;
            if existing.state == ProjectState::Importing { return Err(ProjectError::InUse); }
            if has_blocking_work(&mut tx, actor.user_id, id).await? { return Err(ProjectError::InUse); }
            let row = sqlx::query(&format!("UPDATE factory_identity.projects SET state='deleting',updated_at=clock_timestamp() WHERE id=$1 RETURNING {COLUMNS}"))
                .bind(id).fetch_one(&mut *tx).await.map_err(unavailable)?;
            let result = project(&row)?;
            tx.commit().await.map_err(unavailable)?;
            Ok(result)
        })).await
    }
    async fn finish_delete(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        id: Uuid,
    ) -> Result<(), ProjectError> {
        if id.is_nil() {
            return Err(ProjectError::Invalid);
        }
        with_connection(&self.pool, || ProjectError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, actor, env, generation).await?;
            let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.projects WHERE id=$1 AND user_id=$2 AND environment_id=$3 AND state='deleting')")
                .bind(id).bind(actor.user_id).bind(env).fetch_one(&mut *tx).await.map_err(unavailable)?;
            if !exists { return Err(ProjectError::NotFound); }
            if has_blocking_work(&mut tx, actor.user_id, id).await? { return Err(ProjectError::InUse); }
            sqlx::query("DELETE FROM factory_identity.conversation_attachments WHERE conversation_id IN (SELECT id FROM factory_identity.conversations WHERE project_id=$1 AND user_id=$2)").bind(id).bind(actor.user_id).execute(&mut *tx).await.map_err(unavailable)?;
            sqlx::query("DELETE FROM factory_identity.conversation_requests WHERE run_id IN (SELECT r.id FROM factory_identity.conversation_runs r JOIN factory_identity.conversations c ON c.id=r.conversation_id WHERE c.project_id=$1 AND c.user_id=$2)").bind(id).bind(actor.user_id).execute(&mut *tx).await.map_err(unavailable)?;
            sqlx::query("DELETE FROM factory_identity.conversation_messages WHERE run_id IN (SELECT r.id FROM factory_identity.conversation_runs r JOIN factory_identity.conversations c ON c.id=r.conversation_id WHERE c.project_id=$1 AND c.user_id=$2)").bind(id).bind(actor.user_id).execute(&mut *tx).await.map_err(unavailable)?;
            sqlx::query("DELETE FROM factory_identity.conversation_runs WHERE conversation_id IN (SELECT id FROM factory_identity.conversations WHERE project_id=$1 AND user_id=$2)").bind(id).bind(actor.user_id).execute(&mut *tx).await.map_err(unavailable)?;
            sqlx::query("DELETE FROM factory_identity.conversations WHERE project_id=$1 AND user_id=$2").bind(id).bind(actor.user_id).execute(&mut *tx).await.map_err(unavailable)?;
            sqlx::query("DELETE FROM factory_identity.projects WHERE id=$1 AND user_id=$2 AND state='deleting'").bind(id).bind(actor.user_id).execute(&mut *tx).await.map_err(unavailable)?;
            tx.commit().await.map_err(unavailable)?;
            Ok(())
        })).await
    }
    async fn reserve(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        input: ImportProject,
    ) -> Result<(Project, bool), ProjectError> {
        input.validate()?;
        with_connection(&self.pool, || ProjectError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, actor, env, generation).await?;
            let id = Uuid::new_v4();
            let directory = input.directory()?.to_owned();
            let inserted = sqlx::query(&format!("INSERT INTO factory_identity.projects(id,user_id,repository_id,full_name,cloned_full_name,directory,state,operation_id,environment_id,environment_generation) VALUES($1,$2,$3,$4,$4,$5,'importing',$6,$7,$8) ON CONFLICT (user_id,repository_id) DO NOTHING RETURNING {COLUMNS}"))
                .bind(id).bind(actor.user_id).bind(input.repository_id).bind(&input.full_name).bind(directory).bind(input.operation_id).bind(env).bind(generation)
                .fetch_optional(&mut *tx).await.map_err(|error| {
                    if error.as_database_error().and_then(|e| e.constraint()) == Some("projects_user_directory_key") {
                        ProjectError::DestinationExists
                    } else { unavailable(error) }
                })?;
            let fresh = inserted.is_some();
            let row = if let Some(row) = inserted { row } else {
                sqlx::query(&format!("UPDATE factory_identity.projects SET full_name=$3,updated_at=clock_timestamp() WHERE user_id=$1 AND repository_id=$2 RETURNING {COLUMNS}"))
                    .bind(actor.user_id).bind(input.repository_id).bind(&input.full_name).fetch_one(&mut *tx).await.map_err(unavailable)?
            };
            let project = project(&row)?;
            tx.commit().await.map_err(unavailable)?;
            Ok((project, fresh))
        })).await
    }
    async fn get(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        id: Uuid,
    ) -> Result<Project, ProjectError> {
        if id.is_nil() {
            return Err(ProjectError::Invalid);
        }
        with_connection(
            &self.pool,
            || ProjectError::Unavailable,
            |connection| {
                Box::pin(async move {
                    let mut tx = connection.begin().await.map_err(unavailable)?;
                    authorize(&mut tx, actor, env, generation).await?;
                    let row = sqlx::query(&format!(
                        "SELECT {COLUMNS} FROM factory_identity.projects WHERE id=$1 AND user_id=$2"
                    ))
                    .bind(id)
                    .bind(actor.user_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(unavailable)?
                    .ok_or(ProjectError::NotFound)?;
                    let result = project(&row)?;
                    tx.commit().await.map_err(unavailable)?;
                    Ok(result)
                })
            },
        )
        .await
    }
    async fn list(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        before: Option<Uuid>,
    ) -> Result<ProjectPage, ProjectError> {
        if before.is_some_and(|id| id.is_nil()) {
            return Err(ProjectError::Invalid);
        }
        with_connection(&self.pool, || ProjectError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, actor, env, generation).await?;
            if let Some(cursor) = before {
                let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.projects WHERE id=$1 AND user_id=$2)")
                    .bind(cursor).bind(actor.user_id).fetch_one(&mut *tx).await.map_err(unavailable)?;
                if !exists { return Err(ProjectError::NotFound); }
            }
            let rows = sqlx::query(&format!("SELECT {COLUMNS} FROM factory_identity.projects WHERE user_id=$1 AND ($2::uuid IS NULL OR (created_at,id) < (SELECT created_at,id FROM factory_identity.projects WHERE id=$2 AND user_id=$1)) ORDER BY created_at DESC,id DESC LIMIT $3"))
                .bind(actor.user_id).bind(before).bind(PAGE_SIZE + 1).fetch_all(&mut *tx).await.map_err(unavailable)?;
            let has_more = rows.len() > PAGE_SIZE as usize;
            let items: Vec<Project> = rows.iter().take(PAGE_SIZE as usize).map(project).collect::<Result<_,_>>()?;
            let next_cursor = if has_more { items.last().map(|item| item.id) } else { None };
            tx.commit().await.map_err(unavailable)?;
            Ok(ProjectPage { items, next_cursor })
        })).await
    }
    async fn sync_repository_metadata(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        repositories: &[RepositoryMetadata],
    ) -> Result<Vec<i64>, ProjectError> {
        validate_repository_metadata(repositories)?;
        // Validation rejects contradictory duplicates; collapse identical entries to
        // give UPDATE FROM exactly one deterministic metadata row per repository.
        let entries: std::collections::BTreeMap<i64, String> = repositories
            .iter()
            .map(|repository| (repository.repository_id, repository.full_name.clone()))
            .collect();
        let (ids, names): (Vec<i64>, Vec<String>) = entries.into_iter().unzip();
        with_connection(&self.pool, || ProjectError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, actor, env, generation).await?;
            let mut matched: Vec<i64> = sqlx::query_scalar("UPDATE factory_identity.projects AS p SET full_name=source.full_name, updated_at=CASE WHEN p.full_name <> source.full_name THEN clock_timestamp() ELSE p.updated_at END FROM unnest($2::bigint[], $3::text[]) AS source(repository_id,full_name) WHERE p.user_id=$1 AND p.repository_id=source.repository_id RETURNING p.repository_id")
                .bind(actor.user_id).bind(&ids).bind(&names).fetch_all(&mut *tx).await.map_err(unavailable)?;
            matched.sort_unstable();
            tx.commit().await.map_err(unavailable)?;
            Ok(matched)
        })).await
    }
    async fn settle(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        id: Uuid,
        operation: Uuid,
        success: bool,
    ) -> Result<Project, ProjectError> {
        if id.is_nil() || operation.is_nil() {
            return Err(ProjectError::Invalid);
        }
        with_connection(&self.pool, || ProjectError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, actor, env, generation).await?;
            let state = if success { "ready" } else { "failed" };
            let row = sqlx::query(&format!("UPDATE factory_identity.projects SET state=$6,updated_at=clock_timestamp() WHERE id=$1 AND user_id=$2 AND operation_id=$3 AND environment_id=$4 AND environment_generation=$5 AND state='importing' RETURNING {COLUMNS}"))
                .bind(id).bind(actor.user_id).bind(operation).bind(env).bind(generation).bind(state).fetch_optional(&mut *tx).await.map_err(unavailable)?.ok_or(ProjectError::Conflict)?;
            let result=project(&row)?; tx.commit().await.map_err(unavailable)?; Ok(result)
        })).await
    }
    async fn recover_stopped(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        observed_at: DateTime<Utc>,
        id: Uuid,
    ) -> Result<Project, ProjectError> {
        if id.is_nil() || env.is_nil() || generation <= 0 {
            return Err(ProjectError::Invalid);
        }
        with_connection(&self.pool, || ProjectError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize_actor(&mut tx, actor).await?;
            let stopped: Option<Uuid> = sqlx::query_scalar("SELECT id FROM factory_identity.environments WHERE id=$1 AND user_id=$2 AND generation=$3 AND desired_state='stopped' AND observed_state='stopped' AND observed_generation=$3 AND observed_at=$4 AND error_code IS NULL FOR UPDATE")
                .bind(env).bind(actor.user_id).bind(generation).bind(observed_at).fetch_optional(&mut *tx).await.map_err(unavailable)?;
            if stopped.is_none() { return Err(ProjectError::Conflict); }
            let row = sqlx::query(&format!("UPDATE factory_identity.projects SET state='failed',updated_at=clock_timestamp() WHERE id=$1 AND user_id=$2 AND environment_id=$3 AND environment_generation < $4 AND state='importing' RETURNING {COLUMNS}"))
                .bind(id).bind(actor.user_id).bind(env).bind(generation).fetch_optional(&mut *tx).await.map_err(unavailable)?.ok_or(ProjectError::Conflict)?;
            let result=project(&row)?; tx.commit().await.map_err(unavailable)?; Ok(result)
        })).await
    }
    async fn recoverable(
        &self,
        actor: EnvironmentActor,
    ) -> Result<Vec<RecoverableImport>, ProjectError> {
        with_connection(&self.pool, || ProjectError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize_actor(&mut tx, actor).await?;
            let rows = sqlx::query("SELECT id,operation_id,environment_id,environment_generation FROM factory_identity.projects WHERE user_id=$1 AND state='importing' ORDER BY created_at,id LIMIT $2")
                .bind(actor.user_id).bind(MAX_RECOVERABLE_IMPORTS as i64).fetch_all(&mut *tx).await.map_err(unavailable)?;
            let result = rows.iter().map(|row| Ok(RecoverableImport {
                project_id: row.try_get("id").map_err(unavailable)?,
                operation_id: row.try_get("operation_id").map_err(unavailable)?,
                environment_id: row.try_get("environment_id").map_err(unavailable)?,
                environment_generation: row.try_get("environment_generation").map_err(unavailable)?,
            })).collect::<Result<Vec<_>, ProjectError>>()?;
            tx.commit().await.map_err(unavailable)?;
            Ok(result)
        })).await
    }
    async fn confirm_recovered(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        id: Uuid,
        original_operation: Uuid,
    ) -> Result<Project, ProjectError> {
        if id.is_nil() || original_operation.is_nil() {
            return Err(ProjectError::Invalid);
        }
        with_connection(&self.pool, || ProjectError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, actor, env, generation).await?;
            let row = sqlx::query(&format!("UPDATE factory_identity.projects SET state='ready',updated_at=clock_timestamp() WHERE id=$1 AND user_id=$2 AND operation_id=$3 AND environment_id=$4 AND environment_generation <= $5 AND state='failed' RETURNING {COLUMNS}"))
                .bind(id).bind(actor.user_id).bind(original_operation).bind(env).bind(generation).fetch_optional(&mut *tx).await.map_err(unavailable)?.ok_or(ProjectError::Conflict)?;
            let result=project(&row)?; tx.commit().await.map_err(unavailable)?; Ok(result)
        })).await
    }
    async fn retry(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        id: Uuid,
        previous: Uuid,
        next: Uuid,
    ) -> Result<Project, ProjectError> {
        if id.is_nil() || previous.is_nil() || next.is_nil() || previous == next {
            return Err(ProjectError::Invalid);
        }
        with_connection(&self.pool, || ProjectError::Unavailable, |connection| Box::pin(async move {
            let mut tx = connection.begin().await.map_err(unavailable)?;
            authorize(&mut tx, actor, env, generation).await?;
            let row = sqlx::query(&format!("UPDATE factory_identity.projects SET state='importing',cloned_full_name=full_name,operation_id=$4,environment_id=$5,environment_generation=$6,updated_at=clock_timestamp() WHERE id=$1 AND user_id=$2 AND operation_id=$3 AND state='failed' RETURNING {COLUMNS}"))
                .bind(id).bind(actor.user_id).bind(previous).bind(next).bind(env).bind(generation).fetch_optional(&mut *tx).await.map_err(unavailable)?.ok_or(ProjectError::Conflict)?;
            let result=project(&row)?; tx.commit().await.map_err(unavailable)?; Ok(result)
        })).await
    }
}
