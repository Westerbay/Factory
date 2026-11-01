//! PostgreSQL conversation storage. Every entry point revalidates session and generation.
use crate::connection::with_connection;
use factory_core::{conversations::*, environments::EnvironmentActor};
use sqlx::{Connection, PgPool, Postgres, Row, Transaction, postgres::PgRow};
use uuid::Uuid;

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub struct PgConversationStore {
    pub(crate) pool: PgPool,
}
fn unavailable(_: impl std::fmt::Display) -> ConversationError {
    ConversationError::Unavailable
}
fn conversation(row: &PgRow) -> Result<Conversation, ConversationError> {
    Ok(Conversation {
        id: row.try_get("id").map_err(unavailable)?,
        user_id: row.try_get("user_id").map_err(unavailable)?,
        provider_thread: row.try_get("provider_thread").map_err(unavailable)?,
        created_at: row.try_get("created_at").map_err(unavailable)?,
    })
}
fn status(value: &str) -> Result<RunStatus, ConversationError> {
    match value {
        "active" => Ok(RunStatus::Active),
        "completed" => Ok(RunStatus::Completed),
        "failed" => Ok(RunStatus::Failed),
        "interrupted" => Ok(RunStatus::Interrupted),
        "uncertain" => Ok(RunStatus::Uncertain),
        _ => Err(ConversationError::Unavailable),
    }
}
fn status_str(value: RunStatus) -> &'static str {
    match value {
        RunStatus::Active => "active",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Interrupted => "interrupted",
        RunStatus::Uncertain => "uncertain",
    }
}
fn run(row: &PgRow) -> Result<Run, ConversationError> {
    Ok(Run {
        id: row.try_get("id").map_err(unavailable)?,
        conversation_id: row.try_get("conversation_id").map_err(unavailable)?,
        operation_id: row.try_get("operation_id").map_err(unavailable)?,
        environment_generation: row.try_get("environment_generation").map_err(unavailable)?,
        model: row.try_get("model").map_err(unavailable)?,
        status: status(&row.try_get::<String, _>("status").map_err(unavailable)?)?,
    })
}
fn valid(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.len() <= max && !s.contains('\0')
}
async fn authorize_actor(
    tx: &mut Transaction<'_, Postgres>,
    actor: EnvironmentActor,
) -> Result<(), ConversationError> {
    let active: Option<String> =
        sqlx::query_scalar("SELECT status FROM factory_identity.users WHERE id=$1 FOR UPDATE")
            .bind(actor.user_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(unavailable)?;
    if active.as_deref() != Some("active") {
        return Err(ConversationError::Unauthorized);
    }
    let session: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.sessions WHERE user_id=$1 AND token_hash=$2 AND expires_at>clock_timestamp())")
        .bind(actor.user_id).bind(actor.session_hash.0.as_slice()).fetch_one(&mut **tx).await.map_err(unavailable)?;
    if !session {
        return Err(ConversationError::Unauthorized);
    }
    Ok(())
}
async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    actor: EnvironmentActor,
    env: Uuid,
    generation: i64,
) -> Result<(), ConversationError> {
    authorize_actor(tx, actor).await?;
    let current: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.environments WHERE id=$1 AND user_id=$2 AND generation=$3 AND desired_state='running' AND observed_state='running' AND error_code IS NULL)")
        .bind(env).bind(actor.user_id).bind(generation).fetch_one(&mut **tx).await.map_err(unavailable)?;
    if !current {
        return Err(ConversationError::Conflict);
    }
    Ok(())
}
async fn owned(
    tx: &mut Transaction<'_, Postgres>,
    user: Uuid,
    id: Uuid,
) -> Result<Conversation, ConversationError> {
    let row = sqlx::query("SELECT id,user_id,provider_thread,created_at FROM factory_identity.conversations WHERE id=$1 AND user_id=$2 FOR UPDATE")
        .bind(id).bind(user).fetch_optional(&mut **tx).await.map_err(unavailable)?.ok_or(ConversationError::NotFound)?;
    conversation(&row)
}
async fn owned_run(
    tx: &mut Transaction<'_, Postgres>,
    user: Uuid,
    env: Uuid,
    generation: i64,
    id: Uuid,
) -> Result<Run, ConversationError> {
    let row = sqlx::query("SELECT r.* FROM factory_identity.conversation_runs r JOIN factory_identity.conversations c ON c.id=r.conversation_id WHERE r.id=$1 AND c.user_id=$2 AND r.environment_id=$3 AND r.environment_generation=$4 FOR UPDATE OF r")
        .bind(id).bind(user).bind(env).bind(generation).fetch_optional(&mut **tx).await.map_err(unavailable)?.ok_or(ConversationError::NotFound)?;
    run(&row)
}
async fn require_stopped_proof(
    tx: &mut Transaction<'_, Postgres>,
    actor: EnvironmentActor,
    env: Uuid,
    generation: i64,
    observed_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), ConversationError> {
    let proof: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.environments WHERE id=$1 AND user_id=$2 AND generation=$3 AND desired_state='stopped' AND observed_state='stopped' AND observed_generation=$3 AND observed_at=$4 AND error_code IS NULL)")
        .bind(env).bind(actor.user_id).bind(generation).bind(observed_at).fetch_one(&mut **tx).await.map_err(unavailable)?;
    if !proof {
        return Err(ConversationError::Conflict);
    }
    Ok(())
}
#[async_trait::async_trait]
impl ConversationStore for PgConversationStore {
    async fn reserve_model_probe(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        lease: Uuid,
    ) -> Result<(), ConversationError> {
        if lease.is_nil() {
            return Err(ConversationError::Invalid);
        }
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await?;
                let exists: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM factory_identity.model_probe_leases WHERE user_id=$1)",
                )
                .bind(actor.user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(unavailable)?;
                if exists {
                    return Err(ConversationError::Conflict);
                }
                sqlx::query("INSERT INTO factory_identity.model_probe_leases(id,user_id,environment_id,environment_generation) VALUES($1,$2,$3,$4)").bind(lease).bind(actor.user_id).bind(env).bind(generation).execute(&mut *tx).await.map_err(unavailable)?;
                tx.commit().await.map_err(unavailable)?;
                Ok(())
            })
        })
        .await
    }
    async fn settle_model_probe_launch(
        &self,
        lease: Uuid,
        env: Uuid,
        generation: i64,
    ) -> Result<(), ConversationError> {
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let result = sqlx::query("UPDATE factory_identity.model_probe_leases SET launch_settled=true WHERE id=$1 AND environment_id=$2 AND environment_generation=$3").bind(lease).bind(env).bind(generation).execute(connection).await.map_err(unavailable)?;
                if result.rows_affected() != 1 {
                    return Err(ConversationError::Conflict);
                }
                Ok(())
            })
        })
        .await
    }
    async fn release_model_probe(
        &self,
        actor: EnvironmentActor,
        lease: Uuid,
    ) -> Result<(), ConversationError> {
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize_actor(&mut tx, actor).await?;
                let result = sqlx::query(
                    "DELETE FROM factory_identity.model_probe_leases WHERE id=$1 AND user_id=$2 AND launch_settled",
                )
                .bind(lease)
                .bind(actor.user_id)
                .execute(&mut *tx)
                .await
                .map_err(unavailable)?;
                if result.rows_affected() != 1 {
                    return Err(ConversationError::Conflict);
                }
                tx.commit().await.map_err(unavailable)?;
                Ok(())
            })
        })
        .await
    }
    async fn recover_model_probe_stopped(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, ConversationError> {
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize_actor(&mut tx, actor).await?;
                require_stopped_proof(&mut tx, actor, env, generation, observed_at).await?;
                let result = sqlx::query("DELETE FROM factory_identity.model_probe_leases WHERE user_id=$1 AND environment_id=$2 AND environment_generation < $3 AND launch_settled").bind(actor.user_id).bind(env).bind(generation).execute(&mut *tx).await.map_err(unavailable)?;
                if result.rows_affected() == 0 {
                    let remains: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.model_probe_leases WHERE user_id=$1 AND environment_id=$2)").bind(actor.user_id).bind(env).fetch_one(&mut *tx).await.map_err(unavailable)?;
                    if remains {
                        return Err(ConversationError::Conflict);
                    }
                }
                tx.commit().await.map_err(unavailable)?;
                Ok(result.rows_affected() == 1)
            })
        })
        .await
    }
    async fn has_open_runs(&self, actor: EnvironmentActor) -> Result<bool, ConversationError> {
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize_actor(&mut tx, actor).await?;
                let open=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.conversation_runs r JOIN factory_identity.conversations c ON c.id=r.conversation_id WHERE c.user_id=$1 AND r.status IN ('active','uncertain')) OR EXISTS(SELECT 1 FROM factory_identity.model_probe_leases WHERE user_id=$1)").bind(actor.user_id).fetch_one(&mut *tx).await.map_err(unavailable)?;
                tx.commit().await.map_err(unavailable)?;
                Ok(open)
            })
        })
        .await
    }
    async fn create(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
    ) -> Result<Conversation, ConversationError> {
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await?;
                let row = sqlx::query(
                    "INSERT INTO factory_identity.conversations(id,user_id) VALUES($1,$2) RETURNING *",
                )
                .bind(Uuid::new_v4())
                .bind(actor.user_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(unavailable)?;
                let result = conversation(&row)?;
                tx.commit().await.map_err(unavailable)?;
                Ok(result)
            })
        })
        .await
    }
    async fn list(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        before: Option<Uuid>,
    ) -> Result<ConversationPage, ConversationError> {
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await?;
                // Verify the cursor belongs to this user; a foreign ID never selects another user's rows.
                let cursor = if let Some(id) = before {
                    Some(owned(&mut tx, actor.user_id, id).await?)
                } else {
                    None
                };
                let mut rows = sqlx::query("SELECT * FROM factory_identity.conversations WHERE user_id=$1 AND ($2::timestamptz IS NULL OR (created_at,id)<($2,$3)) ORDER BY created_at DESC,id DESC LIMIT 201")
                    .bind(actor.user_id).bind(cursor.as_ref().map(|c| c.created_at)).bind(before)
                    .fetch_all(&mut *tx).await.map_err(unavailable)?;
                let has_more = rows.len() > 200;
                rows.truncate(200);
                let items = rows
                    .iter()
                    .map(conversation)
                    .collect::<Result<Vec<_>, _>>()?;
                let next_cursor = if has_more {
                    items.last().map(|c| c.id)
                } else {
                    None
                };
                tx.commit().await.map_err(unavailable)?;
                Ok(ConversationPage { items, next_cursor })
            })
        })
        .await
    }
    async fn history(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        id: Uuid,
    ) -> Result<ConversationHistory, ConversationError> {
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await?;
                let conversation = owned(&mut tx, actor.user_id, id).await?;
                // Prioritize active/uncertain work before selecting the bounded window, then
                // present the selected records in chronological order. Fetch one extra row
                // so callers can disclose that older durable history is not shown.
                let mut rows = sqlx::query("SELECT * FROM factory_identity.conversation_runs WHERE conversation_id=$1 ORDER BY (status IN ('active','uncertain')) DESC,created_at DESC,id DESC LIMIT 1001").bind(id).fetch_all(&mut *tx).await.map_err(unavailable)?;
                let runs_truncated = rows.len() > 1000;
                rows.truncate(1000);
                let mut ordered_runs = rows
                    .iter()
                    .map(|row| {
                        Ok((
                            row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                                .map_err(unavailable)?,
                            row.try_get::<Uuid, _>("id").map_err(unavailable)?,
                            run(row)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, ConversationError>>()?;
                ordered_runs.sort_by_key(|(created, id, _)| (*created, *id));
                let runs = ordered_runs.into_iter().map(|(_, _, run)| run).collect();
                let mut rows = sqlx::query("SELECT m.*,r.created_at AS run_created_at FROM factory_identity.conversation_messages m JOIN factory_identity.conversation_runs r ON r.id=m.run_id WHERE r.conversation_id=$1 ORDER BY (r.status IN ('active','uncertain')) DESC,r.created_at DESC,r.id DESC,m.ordinal DESC LIMIT 2001").bind(id).fetch_all(&mut *tx).await.map_err(unavailable)?;
                let messages_truncated = rows.len() > 2000;
                rows.truncate(2000);
                let mut ordered_messages = rows
                    .iter()
                    .map(|r| {
                        Ok((
                            r.try_get::<chrono::DateTime<chrono::Utc>, _>("run_created_at")
                                .map_err(unavailable)?,
                            r.try_get::<Uuid, _>("run_id").map_err(unavailable)?,
                            r.try_get::<i64, _>("ordinal").map_err(unavailable)?,
                            Message {
                                id: r.try_get("id").map_err(unavailable)?,
                                run_id: r.try_get("run_id").map_err(unavailable)?,
                                role: match r
                                    .try_get::<String, _>("role")
                                    .map_err(unavailable)?
                                    .as_str()
                                {
                                    "user" => MessageRole::User,
                                    "assistant" => MessageRole::Assistant,
                                    _ => return Err(ConversationError::Unavailable),
                                },
                                content: r.try_get("content").map_err(unavailable)?,
                                created_at: r.try_get("created_at").map_err(unavailable)?,
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>, ConversationError>>()?;
                ordered_messages.sort_by_key(|(created, id, ordinal, _)| (*created, *id, *ordinal));
                let messages = ordered_messages
                    .into_iter()
                    .map(|(_, _, _, message)| message)
                    .collect();
                let mut rows = sqlx::query("SELECT q.* FROM factory_identity.conversation_requests q JOIN factory_identity.conversation_runs r ON r.id=q.run_id WHERE r.conversation_id=$1 AND q.response_state IN ('pending','claimed','uncertain') ORDER BY (r.status IN ('active','uncertain')) DESC,q.created_at DESC,q.id DESC LIMIT 101").bind(id).fetch_all(&mut *tx).await.map_err(unavailable)?;
                let requests_truncated = rows.len() > 100;
                rows.truncate(100);
                let mut ordered_requests = rows
                    .iter()
                    .map(|r| {
                        Ok((
                            r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                                .map_err(unavailable)?,
                            r.try_get::<Uuid, _>("id").map_err(unavailable)?,
                            PendingRequest {
                                id: r.try_get("id").map_err(unavailable)?,
                                run_id: r.try_get("run_id").map_err(unavailable)?,
                                kind: match r
                                    .try_get::<String, _>("kind")
                                    .map_err(unavailable)?
                                    .as_str()
                                {
                                    "permission" => RequestKind::Permission,
                                    "question" => RequestKind::Question,
                                    _ => return Err(ConversationError::Unavailable),
                                },
                                state: match r
                                    .try_get::<String, _>("response_state")
                                    .map_err(unavailable)?
                                    .as_str()
                                {
                                    "pending" => RequestState::Pending,
                                    "claimed" => RequestState::Claimed,
                                    "uncertain" => RequestState::Uncertain,
                                    _ => return Err(ConversationError::Unavailable),
                                },
                                details: r.try_get("details").map_err(unavailable)?,
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>, ConversationError>>()?;
                ordered_requests.sort_by_key(|(created, id, _)| (*created, *id));
                let requests = ordered_requests
                    .into_iter()
                    .map(|(_, _, request)| request)
                    .collect();
                tx.commit().await.map_err(unavailable)?;
                Ok(ConversationHistory {
                    conversation,
                    runs,
                    messages,
                    requests,
                    runs_truncated,
                    messages_truncated,
                    requests_truncated,
                })
            })
        })
        .await
    }
    async fn begin_turn(
        &self,
        actor: EnvironmentActor,
        turn: StartTurn,
    ) -> Result<(Run, bool), ConversationError> {
        if turn.operation_id.is_nil() || !valid(&turn.input, 65536) || !valid(&turn.model, 256) {
            return Err(ConversationError::Invalid);
        }
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, turn.environment_id, turn.generation).await?;
                owned(&mut tx, actor.user_id, turn.conversation_id).await?;
                let prior = sqlx::query("SELECT * FROM factory_identity.conversation_runs WHERE conversation_id=$1 AND operation_id=$2").bind(turn.conversation_id).bind(turn.operation_id).fetch_optional(&mut *tx).await.map_err(unavailable)?;
                if let Some(row) = prior {
                    let previous = run(&row)?;
                    let input: String = row.try_get("input").map_err(unavailable)?;
                    let env: Uuid = row.try_get("environment_id").map_err(unavailable)?;
                    if previous.model != turn.model
                        || input != turn.input
                        || env != turn.environment_id
                        || previous.environment_generation != turn.generation
                    {
                        return Err(ConversationError::Conflict);
                    }
                    tx.commit().await.map_err(unavailable)?;
                    return Ok((previous, false));
                }
                let busy:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.conversation_runs WHERE conversation_id=$1 AND status IN ('active','uncertain'))").bind(turn.conversation_id).fetch_one(&mut *tx).await.map_err(unavailable)?;
                if busy {
                    return Err(ConversationError::Conflict);
                }
                let row=sqlx::query("INSERT INTO factory_identity.conversation_runs(id,conversation_id,operation_id,environment_id,environment_generation,model,input,status) VALUES($1,$2,$3,$4,$5,$6,$7,'active') RETURNING *").bind(Uuid::new_v4()).bind(turn.conversation_id).bind(turn.operation_id).bind(turn.environment_id).bind(turn.generation).bind(&turn.model).bind(&turn.input).fetch_one(&mut *tx).await.map_err(unavailable)?;
                let result = run(&row)?;
                sqlx::query("INSERT INTO factory_identity.conversation_messages(id,run_id,role,content,ordinal) VALUES($1,$2,'user',$3,0)").bind(Uuid::new_v4()).bind(result.id).bind(&turn.input).execute(&mut *tx).await.map_err(unavailable)?;
                tx.commit().await.map_err(unavailable)?;
                Ok((result, true))
            })
        })
        .await
    }
    async fn settle_run_launch(
        &self,
        run_id: Uuid,
        env: Uuid,
        generation: i64,
    ) -> Result<(), ConversationError> {
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let result = sqlx::query("UPDATE factory_identity.conversation_runs SET launch_settled=true WHERE id=$1 AND environment_id=$2 AND environment_generation=$3 AND status IN ('active','uncertain')").bind(run_id).bind(env).bind(generation).execute(connection).await.map_err(unavailable)?;
                if result.rows_affected() != 1 {
                    return Err(ConversationError::Conflict);
                }
                Ok(())
            })
        })
        .await
    }
    async fn set_thread(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        run_id: Uuid,
        thread: &str,
        previous: Option<&str>,
    ) -> Result<(), ConversationError> {
        if !valid(thread, 512) {
            return Err(ConversationError::Invalid);
        }
        let thread = thread.to_owned();
        let previous = previous.map(str::to_owned);
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await?;
                let run = owned_run(&mut tx, actor.user_id, env, generation, run_id).await?;
                if run.status != RunStatus::Active {
                    return Err(ConversationError::Conflict);
                }
                let updated=sqlx::query("UPDATE factory_identity.conversations SET provider_thread=$1 WHERE id=$2 AND user_id=$3 AND (provider_thread IS NOT DISTINCT FROM $4 OR provider_thread=$1)").bind(thread).bind(run.conversation_id).bind(actor.user_id).bind(previous).execute(&mut *tx).await.map_err(unavailable)?;
                if updated.rows_affected() != 1 {
                    return Err(ConversationError::Conflict);
                }
                tx.commit().await.map_err(unavailable)?;
                Ok(())
            })
        })
        .await
    }
    async fn append_assistant(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        run_id: Uuid,
        provider_item: &str,
        text: &str,
    ) -> Result<(Message, bool), ConversationError> {
        if !valid(provider_item, 512)
            || text.is_empty()
            || text.len() > 262144
            || text.contains('\0')
        {
            return Err(ConversationError::Invalid);
        }
        let provider_item = provider_item.to_owned();
        let text = text.to_owned();
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await?;
                let run = owned_run(&mut tx, actor.user_id, env, generation, run_id).await?;
                let previous=sqlx::query("SELECT * FROM factory_identity.conversation_messages WHERE run_id=$1 AND provider_item=$2").bind(run_id).bind(&provider_item).fetch_optional(&mut *tx).await.map_err(unavailable)?;
                if let Some(row) = previous {
                    let content: String = row.try_get("content").map_err(unavailable)?;
                    if content != text {
                        return Err(ConversationError::Conflict);
                    }
                    let message = Message {
                        id: row.try_get("id").map_err(unavailable)?,
                        run_id,
                        role: MessageRole::Assistant,
                        content,
                        created_at: row.try_get("created_at").map_err(unavailable)?,
                    };
                    tx.commit().await.map_err(unavailable)?;
                    return Ok((message, false));
                }
                if run.status != RunStatus::Active {
                    return Err(ConversationError::Conflict);
                }
                let row=sqlx::query("INSERT INTO factory_identity.conversation_messages(id,run_id,role,content,ordinal,provider_item) SELECT $1,$2,'assistant',$3,COALESCE(MAX(ordinal),-1)+1,$4 FROM factory_identity.conversation_messages WHERE run_id=$2 RETURNING id,created_at")
                    .bind(Uuid::new_v4()).bind(run_id).bind(&text).bind(&provider_item).fetch_one(&mut *tx).await.map_err(unavailable)?;
                let message = Message {
                    id: row.try_get("id").map_err(unavailable)?,
                    run_id,
                    role: MessageRole::Assistant,
                    content: text,
                    created_at: row.try_get("created_at").map_err(unavailable)?,
                };
                tx.commit().await.map_err(unavailable)?;
                Ok((message, true))
            })
        })
        .await
    }
    async fn add_request(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        request: PendingRequest,
        provider_reference: &str,
    ) -> Result<(), ConversationError> {
        if request.id.is_nil()
            || request.state != RequestState::Pending
            || !valid(&request.details, 16384)
            || !valid(provider_reference, 512)
        {
            return Err(ConversationError::Invalid);
        }
        let provider_reference = provider_reference.to_owned();
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await?;
                let run = owned_run(&mut tx, actor.user_id, env, generation, request.run_id).await?;
                if run.status != RunStatus::Active {
                    return Err(ConversationError::Conflict);
                }
                let kind = match request.kind {
                    RequestKind::Permission => "permission",
                    RequestKind::Question => "question",
                };
                let row=sqlx::query("INSERT INTO factory_identity.conversation_requests(id,run_id,kind,details,provider_reference) VALUES($1,$2,$3,$4,$5) ON CONFLICT(id) DO UPDATE SET id=EXCLUDED.id WHERE factory_identity.conversation_requests.run_id=EXCLUDED.run_id AND factory_identity.conversation_requests.kind=EXCLUDED.kind AND factory_identity.conversation_requests.details=EXCLUDED.details AND factory_identity.conversation_requests.provider_reference=EXCLUDED.provider_reference AND factory_identity.conversation_requests.response_state='pending'")
                    .bind(request.id).bind(request.run_id).bind(kind).bind(&request.details).bind(provider_reference).execute(&mut *tx).await.map_err(unavailable)?;
                if row.rows_affected() != 1 {
                    return Err(ConversationError::Conflict);
                }
                tx.commit().await.map_err(unavailable)?;
                Ok(())
            })
        })
        .await
    }
    async fn claim_response(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        request_id: Uuid,
        operation_id: Uuid,
        answer: &str,
    ) -> Result<ResponseClaim, ConversationError> {
        if operation_id.is_nil() || !valid(answer, 16384) {
            return Err(ConversationError::Invalid);
        }
        let answer = answer.to_owned();
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await?;
                let row = sqlx::query("SELECT q.* FROM factory_identity.conversation_requests q JOIN factory_identity.conversation_runs r ON r.id=q.run_id JOIN factory_identity.conversations c ON c.id=r.conversation_id WHERE q.id=$1 AND c.user_id=$2 AND r.environment_id=$3 AND r.environment_generation=$4 AND r.status='active' FOR UPDATE OF q")
                    .bind(request_id).bind(actor.user_id).bind(env).bind(generation).fetch_optional(&mut *tx).await.map_err(unavailable)?.ok_or(ConversationError::NotFound)?;
                let existing: String = row.try_get("response_state").map_err(unavailable)?;
                let reference: String = row.try_get("provider_reference").map_err(unavailable)?;
                let fresh = existing == "pending";
                let state = match existing.as_str() {
                    "pending" => {
                        sqlx::query("UPDATE factory_identity.conversation_requests SET response_state='claimed',response_operation_id=$2,response_answer=$3 WHERE id=$1").bind(request_id).bind(operation_id).bind(answer).execute(&mut *tx).await.map_err(unavailable)?;
                        ResponseState::Claimed
                    }
                    "claimed" | "delivered" | "uncertain" => {
                        let previous: Option<Uuid> =
                            row.try_get("response_operation_id").map_err(unavailable)?;
                        let previous_answer: Option<String> =
                            row.try_get("response_answer").map_err(unavailable)?;
                        if previous != Some(operation_id) || previous_answer.as_deref() != Some(answer.as_str()) {
                            return Err(ConversationError::Conflict);
                        }
                        match existing.as_str() {
                            "claimed" => ResponseState::Claimed,
                            "delivered" => ResponseState::Delivered,
                            _ => ResponseState::Uncertain,
                        }
                    }
                    _ => return Err(ConversationError::Conflict),
                };
                tx.commit().await.map_err(unavailable)?;
                Ok(ResponseClaim {
                    provider_reference: reference,
                    fresh,
                    state,
                })
            })
        })
        .await
    }
    async fn record_response(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        request_id: Uuid,
        operation_id: Uuid,
        state: ResponseState,
    ) -> Result<(), ConversationError> {
        let result_state = match state {
            ResponseState::Delivered => "delivered",
            ResponseState::Uncertain => "uncertain",
            ResponseState::Claimed => return Err(ConversationError::Invalid),
        };
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await?;
                let result=sqlx::query("UPDATE factory_identity.conversation_requests q SET response_state=$6 FROM factory_identity.conversation_runs r, factory_identity.conversations c WHERE q.id=$1 AND q.response_operation_id=$2 AND (q.response_state='claimed' OR (q.response_state='uncertain' AND $6='delivered')) AND q.run_id=r.id AND r.conversation_id=c.id AND c.user_id=$3 AND r.environment_id=$4 AND r.environment_generation=$5 AND r.status='active'")
                    .bind(request_id).bind(operation_id).bind(actor.user_id).bind(env).bind(generation).bind(result_state).execute(&mut *tx).await.map_err(unavailable)?;
                if result.rows_affected() != 1 {
                    return Err(ConversationError::Conflict);
                }
                tx.commit().await.map_err(unavailable)?;
                Ok(())
            })
        })
        .await
    }
    async fn recoverable_stopped(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<RecoverableRun>, ConversationError> {
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize_actor(&mut tx, actor).await?;
                require_stopped_proof(&mut tx, actor, env, generation, observed_at).await?;
                let rows = sqlx::query("SELECT r.id AS run_id, r.conversation_id, r.launch_settled FROM factory_identity.conversation_runs r JOIN factory_identity.conversations c ON c.id=r.conversation_id WHERE c.user_id=$1 AND r.environment_id=$2 AND r.environment_generation < $3 AND r.status IN ('active','uncertain') ORDER BY r.created_at,r.id LIMIT 200")
                    .bind(actor.user_id).bind(env).bind(generation).fetch_all(&mut *tx).await.map_err(unavailable)?;
                let result = rows
                    .iter()
                    .map(|row| {
                        Ok(RecoverableRun {
                            run_id: row.try_get("run_id").map_err(unavailable)?,
                            conversation_id: row.try_get("conversation_id").map_err(unavailable)?,
                            launch_settled: row.try_get("launch_settled").map_err(unavailable)?,
                        })
                    })
                    .collect::<Result<Vec<_>, ConversationError>>()?;
                tx.commit().await.map_err(unavailable)?;
                Ok(result)
            })
        })
        .await
    }
    async fn recover_stopped(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        observed_at: chrono::DateTime<chrono::Utc>,
        run_id: Uuid,
    ) -> Result<(), ConversationError> {
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize_actor(&mut tx, actor).await?;
                // The user lock serializes start/stop with this proof and mutation. An observation
                // from before the latest intent, or a different supervisor inspection, is not proof.
                require_stopped_proof(&mut tx, actor, env, generation, observed_at).await?;
                let result=sqlx::query("UPDATE factory_identity.conversation_runs r SET status='interrupted' FROM factory_identity.conversations c WHERE r.id=$1 AND r.conversation_id=c.id AND c.user_id=$2 AND r.environment_id=$3 AND r.environment_generation < $4 AND r.launch_settled AND r.status IN ('active','uncertain')")
                    .bind(run_id).bind(actor.user_id).bind(env).bind(generation).execute(&mut *tx).await.map_err(unavailable)?;
                if result.rows_affected() != 1 {
                    return Err(ConversationError::Conflict);
                }
                sqlx::query("UPDATE factory_identity.conversation_requests SET response_state=CASE WHEN response_state='pending' THEN 'expired' ELSE 'uncertain' END WHERE run_id=$1 AND response_state IN ('pending','claimed')")
                    .bind(run_id).execute(&mut *tx).await.map_err(unavailable)?;
                tx.commit().await.map_err(unavailable)?;
                Ok(())
            })
        })
        .await
    }
    async fn finish_turn(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        run_id: Uuid,
        status: RunStatus,
    ) -> Result<(), ConversationError> {
        if status == RunStatus::Active {
            return Err(ConversationError::Invalid);
        }
        with_connection(&self.pool, || ConversationError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await?;
                let run = owned_run(&mut tx, actor.user_id, env, generation, run_id).await?;
                if !matches!(run.status, RunStatus::Active | RunStatus::Uncertain) {
                    return Err(ConversationError::Conflict);
                }
                let updated = sqlx::query("UPDATE factory_identity.conversation_runs SET status=$1 WHERE id=$2 AND (launch_settled OR $1='uncertain')")
                    .bind(status_str(status))
                    .bind(run_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(unavailable)?;
                if updated.rows_affected() != 1 {
                    return Err(ConversationError::Conflict);
                }
                if status != RunStatus::Uncertain {
                    sqlx::query("UPDATE factory_identity.conversation_requests SET response_state=CASE WHEN response_state='pending' THEN 'expired' ELSE 'uncertain' END WHERE run_id=$1 AND response_state IN ('pending','claimed')").bind(run_id).execute(&mut *tx).await.map_err(unavailable)?;
                }
                tx.commit().await.map_err(unavailable)?;
                Ok(())
            })
        })
        .await
    }
}
