//! Durable feature state and operation receipts in the owned Factory database.
mod records;

use crate::connection::with_connection;
use chrono::{DateTime, Utc};
use factory_core::{
    environments::{Environment, EnvironmentActor},
    features::*,
};
use sqlx::{Connection, PgPool, Postgres, Row, Transaction, postgres::PgRow, types::Json};
use uuid::Uuid;

const PAGE_SIZE: i64 = 100;

#[derive(Clone)]
pub struct PgFeatureStore {
    pool: PgPool,
}

impl PgFeatureStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn unavailable(_: impl std::fmt::Display) -> FeatureError {
    FeatureError::Unavailable
}

fn stage_name(stage: FeatureStage) -> &'static str {
    match stage {
        FeatureStage::Scope => "scope",
        FeatureStage::Architecture => "architecture",
        FeatureStage::Execution => "execution",
        FeatureStage::Review => "review",
        FeatureStage::Result => "result",
        FeatureStage::Draft => "draft",
    }
}

fn feature(row: &PgRow) -> Result<Feature, FeatureError> {
    let stage: &str = row.try_get("stage").map_err(unavailable)?;
    Ok(Feature {
        id: row.try_get("id").map_err(unavailable)?,
        user_id: row.try_get("user_id").map_err(unavailable)?,
        project_id: row.try_get("project_id").map_err(unavailable)?,
        conversation_id: row.try_get("conversation_id").map_err(unavailable)?,
        environment_id: row.try_get("environment_id").map_err(unavailable)?,
        provider: row.try_get("provider").map_err(unavailable)?,
        workflow_revision: row.try_get("workflow_revision").map_err(unavailable)?,
        title: row.try_get("title").map_err(unavailable)?,
        stage: match stage {
            "scope" => FeatureStage::Scope,
            "architecture" => FeatureStage::Architecture,
            "execution" => FeatureStage::Execution,
            "review" => FeatureStage::Review,
            "result" => FeatureStage::Result,
            "draft" => FeatureStage::Draft,
            _ => return Err(FeatureError::Unavailable),
        },
        created_at: row.try_get("created_at").map_err(unavailable)?,
    })
}

async fn authorize_actor(
    tx: &mut Transaction<'_, Postgres>,
    actor: EnvironmentActor,
) -> Result<(), FeatureError> {
    // Serialize with environment intent, suspension and project deletion on the user.
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM factory_identity.users WHERE id=$1 FOR UPDATE")
            .bind(actor.user_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(unavailable)?;
    if status.as_deref() != Some("active") {
        return Err(FeatureError::Unauthorized);
    }
    // Logout deletes the session without locking the user.
    let active: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM factory_identity.sessions
         WHERE user_id=$1 AND token_hash=$2 AND expires_at>clock_timestamp()
         FOR SHARE",
    )
    .bind(actor.user_id)
    .bind(actor.session_hash.0.as_slice())
    .fetch_optional(&mut **tx)
    .await
    .map_err(unavailable)?;
    active.ok_or(FeatureError::Unauthorized).map(|_| ())
}

#[derive(Clone, Copy)]
struct CurrentEnvironment {
    id: Uuid,
    generation: i64,
}

impl CurrentEnvironment {
    fn new(actor: EnvironmentActor, environment: &Environment) -> Result<Self, FeatureError> {
        if environment.user_id != actor.user_id {
            return Err(FeatureError::Unauthorized);
        }
        if environment.id.is_nil() || environment.generation <= 0 {
            return Err(FeatureError::Invalid);
        }
        Ok(Self {
            id: environment.id,
            generation: environment.generation,
        })
    }
}

async fn require_environment(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    environment: CurrentEnvironment,
    allow_stopped: bool,
) -> Result<(), FeatureError> {
    let current: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM factory_identity.environments
         WHERE id=$1 AND user_id=$2 AND generation=$3 AND observed_generation=$3
           AND error_code IS NULL AND desired_state=observed_state
           AND (desired_state='running' OR ($4 AND desired_state='stopped'))
         FOR UPDATE",
    )
    .bind(environment.id)
    .bind(user_id)
    .bind(environment.generation)
    .bind(allow_stopped)
    .fetch_optional(&mut **tx)
    .await
    .map_err(unavailable)?;
    current.ok_or(FeatureError::Conflict).map(|_| ())
}

async fn require_project(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    environment_id: Uuid,
    project_id: Uuid,
) -> Result<(), FeatureError> {
    let project: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM factory_identity.projects
         WHERE id=$1 AND user_id=$2 AND environment_id=$3 AND state='ready'
         FOR SHARE",
    )
    .bind(project_id)
    .bind(user_id)
    .bind(environment_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(unavailable)?;
    project.ok_or(FeatureError::NotFound).map(|_| ())
}

async fn open_conversation(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    environment: CurrentEnvironment,
    request: &NewFeature,
) -> Result<Uuid, FeatureError> {
    let conversation_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO factory_identity.conversations(id,user_id,project_id) VALUES($1,$2,$3)",
    )
    .bind(conversation_id)
    .bind(user_id)
    .bind(request.project_id)
    .execute(&mut **tx)
    .await
    .map_err(unavailable)?;
    // The opening message is persisted history. It has no model process or fake user message.
    sqlx::query(
        "INSERT INTO factory_identity.conversation_runs
         (id,conversation_id,operation_id,environment_id,environment_generation,model,input,status,launch_settled)
         VALUES($1,$2,$3,$4,$5,$6,'Feature opened','completed',true)",
    )
    .bind(run_id)
    .bind(conversation_id)
    .bind(request.operation_id)
    .bind(environment.id)
    .bind(environment.generation)
    .bind(format!("factory:{}:feature-opener", request.provider))
    .execute(&mut **tx)
    .await
    .map_err(unavailable)?;
    sqlx::query(
        "INSERT INTO factory_identity.conversation_messages(id,run_id,role,content,ordinal)
         VALUES($1,$2,'assistant',$3,0)",
    )
    .bind(Uuid::new_v4())
    .bind(run_id)
    .bind(&request.opener)
    .execute(&mut **tx)
    .await
    .map_err(unavailable)?;
    Ok(conversation_id)
}

#[async_trait::async_trait]
impl FeatureStore for PgFeatureStore {
    async fn create(
        &self,
        actor: EnvironmentActor,
        environment: &Environment,
        request: NewFeature,
    ) -> Result<FeatureState, FeatureError> {
        request.validate()?;
        let environment = CurrentEnvironment::new(actor, environment)?;
        with_connection(&self.pool, || FeatureError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize_actor(&mut tx, actor).await?;
                require_environment(&mut tx, actor.user_id, environment, false).await?;
                require_project(&mut tx, actor.user_id, environment.id, request.project_id).await?;
                let previous = sqlx::query(
                    "SELECT * FROM factory_identity.features WHERE user_id=$1 AND create_operation_id=$2",
                )
                .bind(actor.user_id)
                .bind(request.operation_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(unavailable)?;
                let state = if let Some(row) = previous {
                    let Json(old_request): Json<NewFeature> = row.try_get("create_request").map_err(unavailable)?;
                    if old_request != request || feature(&row)?.environment_id != environment.id {
                        return Err(FeatureError::Conflict);
                    }
                    records::load(&mut tx, &row).await?
                } else {
                    let conversation_id = open_conversation(&mut tx, actor.user_id, environment, &request).await?;
                    let row = sqlx::query(
                        "INSERT INTO factory_identity.features
                         (id,user_id,project_id,conversation_id,environment_id,create_operation_id,create_request,provider,workflow_revision,title,stage)
                         VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'scope') RETURNING *",
                    )
                    .bind(Uuid::new_v4())
                    .bind(actor.user_id)
                    .bind(request.project_id)
                    .bind(conversation_id)
                    .bind(environment.id)
                    .bind(request.operation_id)
                    .bind(Json(&request))
                    .bind(&request.provider)
                    .bind(&request.workflow_revision)
                    .bind(&request.title)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(unavailable)?;
                    FeatureState::new(feature(&row)?)
                };
                tx.commit().await.map_err(unavailable)?;
                Ok(state)
            })
        }).await
    }

    async fn list(
        &self,
        actor: EnvironmentActor,
        project_id: Option<Uuid>,
        before: Option<Uuid>,
    ) -> Result<FeaturePage, FeatureError> {
        with_connection(
            &self.pool,
            || FeatureError::Unavailable,
            |connection| {
                Box::pin(async move {
                    let mut tx = connection.begin().await.map_err(unavailable)?;
                    authorize_actor(&mut tx, actor).await?;
                    let cursor = if let Some(id) = before {
                        let created_at = sqlx::query_scalar::<_, DateTime<Utc>>(
                            "SELECT created_at FROM factory_identity.features
                         WHERE user_id=$1 AND id=$2 AND ($3::uuid IS NULL OR project_id=$3)",
                        )
                        .bind(actor.user_id)
                        .bind(id)
                        .bind(project_id)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(unavailable)?
                        .ok_or(FeatureError::NotFound)?;
                        Some(created_at)
                    } else {
                        None
                    };
                    let mut rows = sqlx::query(
                        "SELECT * FROM factory_identity.features WHERE user_id=$1
                       AND ($2::uuid IS NULL OR project_id=$2)
                       AND ($3::timestamptz IS NULL OR (created_at,id)<($3,$4))
                     ORDER BY created_at DESC,id DESC LIMIT $5",
                    )
                    .bind(actor.user_id)
                    .bind(project_id)
                    .bind(cursor)
                    .bind(before)
                    .bind(PAGE_SIZE + 1)
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(unavailable)?;
                    let has_more = rows.len() > PAGE_SIZE as usize;
                    rows.truncate(PAGE_SIZE as usize);
                    let items = rows.iter().map(feature).collect::<Result<Vec<_>, _>>()?;
                    let next_cursor = items.last().filter(|_| has_more).map(|last| last.id);
                    tx.commit().await.map_err(unavailable)?;
                    Ok(FeaturePage { items, next_cursor })
                })
            },
        )
        .await
    }

    async fn get(&self, actor: EnvironmentActor, id: Uuid) -> Result<FeatureState, FeatureError> {
        self.read(
            actor,
            id,
            "SELECT * FROM factory_identity.features WHERE user_id=$1 AND id=$2",
        )
        .await?
        .ok_or(FeatureError::NotFound)
    }

    async fn get_by_conversation(
        &self,
        actor: EnvironmentActor,
        conversation_id: Uuid,
    ) -> Result<Option<FeatureState>, FeatureError> {
        self.read(
            actor,
            conversation_id,
            "SELECT * FROM factory_identity.features WHERE user_id=$1 AND conversation_id=$2",
        )
        .await
    }

    async fn apply(
        &self,
        actor: EnvironmentActor,
        environment: &Environment,
        id: Uuid,
        operation_id: Uuid,
        command: FeatureCommand,
    ) -> Result<FeatureReceipt, FeatureError> {
        if id.is_nil() || operation_id.is_nil() {
            return Err(FeatureError::Invalid);
        }
        let environment = CurrentEnvironment::new(actor, environment)?;
        with_connection(&self.pool, || FeatureError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize_actor(&mut tx, actor).await?;
                let recovery = matches!(&command, FeatureCommand::RecoverAttempt { .. });
                require_environment(&mut tx, actor.user_id, environment, recovery).await?;
                let row = sqlx::query(
                    "SELECT * FROM factory_identity.features WHERE user_id=$1 AND id=$2 AND environment_id=$3 FOR UPDATE",
                )
                .bind(actor.user_id)
                .bind(id)
                .bind(environment.id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(unavailable)?
                .ok_or(FeatureError::NotFound)?;
                let previous = records::load(&mut tx, &row).await?;
                require_project(&mut tx, actor.user_id, environment.id, previous.feature.project_id).await?;
                let recorded = sqlx::query_scalar::<_, Json<FeatureCommand>>(
                    "SELECT command FROM factory_identity.feature_operations WHERE feature_id=$1 AND id=$2",
                )
                .bind(id)
                .bind(operation_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(unavailable)?;
                if let Some(Json(recorded)) = recorded {
                    if recorded != command {
                        return Err(FeatureError::Conflict);
                    }
                    tx.commit().await.map_err(unavailable)?;
                    return Ok(FeatureReceipt { state: previous, fresh: false });
                }
                let now = sqlx::query_scalar("SELECT clock_timestamp()").fetch_one(&mut *tx).await.map_err(unavailable)?;
                let mut state = previous.clone();
                state.apply(actor.user_id, environment.generation, operation_id, command.clone(), now)?;
                records::update(&mut tx, &previous, &state).await?;
                sqlx::query("INSERT INTO factory_identity.feature_operations(feature_id,id,command) VALUES($1,$2,$3)")
                    .bind(id)
                    .bind(operation_id)
                    .bind(Json(command))
                    .execute(&mut *tx)
                    .await
                    .map_err(unavailable)?;
                tx.commit().await.map_err(unavailable)?;
                Ok(FeatureReceipt { state, fresh: true })
            })
        })
        .await
    }
}

impl PgFeatureStore {
    async fn read(
        &self,
        actor: EnvironmentActor,
        id: Uuid,
        select: &'static str,
    ) -> Result<Option<FeatureState>, FeatureError> {
        with_connection(
            &self.pool,
            || FeatureError::Unavailable,
            |connection| {
                Box::pin(async move {
                    let mut tx = connection.begin().await.map_err(unavailable)?;
                    authorize_actor(&mut tx, actor).await?;
                    let row = sqlx::query(select)
                        .bind(actor.user_id)
                        .bind(id)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(unavailable)?;
                    let state = if let Some(row) = row {
                        Some(records::load(&mut tx, &row).await?)
                    } else {
                        None
                    };
                    tx.commit().await.map_err(unavailable)?;
                    Ok(state)
                })
            },
        )
        .await
    }
}
