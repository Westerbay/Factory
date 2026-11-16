//! PostgreSQL attachment metadata. Every entry point revalidates session, ownership and
//! environment generation; bytes are never stored here.
use crate::connection::with_connection;
use crate::conversations::{authorize, authorize_actor, owned, owned_run};
use chrono::{DateTime, Utc};
use factory_core::{
    attachments::*, conversations::ConversationError, environments::EnvironmentActor,
};
use sqlx::{Connection, PgConnection, PgPool, Row, postgres::PgRow};
use uuid::Uuid;

/// Bounds one unfiltered listing and one expiry sweep.
const LIST_LIMIT: i64 = 1000;
const EXPIRE_BATCH: i64 = 100;
const UNAVAILABLE_CODE: &str = "unavailable";

#[derive(Clone)]
pub struct PgAttachmentStore {
    pool: PgPool,
}

impl PgAttachmentStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn unavailable(_: impl std::fmt::Display) -> AttachmentError {
    AttachmentError::Unavailable
}

fn from_conversation(error: ConversationError) -> AttachmentError {
    match error {
        ConversationError::Unauthorized => AttachmentError::Unauthorized,
        ConversationError::NotFound => AttachmentError::NotFound,
        ConversationError::Conflict => AttachmentError::Conflict,
        ConversationError::Invalid => AttachmentError::Invalid,
        ConversationError::ContextLimit | ConversationError::Unavailable => {
            AttachmentError::Unavailable
        }
    }
}

pub(crate) const COLUMNS: &str =
    "id,conversation_id,run_id,message_id,source,state,name,media_type,kind,size_bytes,created_at";

pub(crate) fn attachment(row: &PgRow) -> Result<Attachment, AttachmentError> {
    let size: i64 = row.try_get("size_bytes").map_err(unavailable)?;
    Ok(Attachment {
        id: row.try_get("id").map_err(unavailable)?,
        conversation_id: row.try_get("conversation_id").map_err(unavailable)?,
        run_id: row.try_get("run_id").map_err(unavailable)?,
        message_id: row.try_get("message_id").map_err(unavailable)?,
        source: AttachmentSource::parse(&row.try_get::<String, _>("source").map_err(unavailable)?)
            .ok_or(AttachmentError::Unavailable)?,
        state: AttachmentState::parse(&row.try_get::<String, _>("state").map_err(unavailable)?)
            .ok_or(AttachmentError::Unavailable)?,
        name: row.try_get("name").map_err(unavailable)?,
        media_type: row.try_get("media_type").map_err(unavailable)?,
        kind: MediaKind::parse(&row.try_get::<String, _>("kind").map_err(unavailable)?)
            .ok_or(AttachmentError::Unavailable)?,
        size: u64::try_from(size).map_err(unavailable)?,
        created_at: row.try_get("created_at").map_err(unavailable)?,
    })
}

fn valid_file(file: &StoredFile) -> Result<i64, AttachmentError> {
    if file.size > MAX_ATTACHMENT_BYTES {
        return Err(AttachmentError::TooLarge);
    }
    if file.media_type.is_empty()
        || file.media_type.len() > 255
        || file.sha256.len() != 64
        || !file
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(AttachmentError::Invalid);
    }
    i64::try_from(file.size).map_err(|_| AttachmentError::TooLarge)
}

/// The conversation must be owned, and its project (if any) still usable.
async fn usable_conversation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: EnvironmentActor,
    env: Uuid,
    id: Uuid,
) -> Result<(), AttachmentError> {
    let conversation = owned(tx, actor.user_id, id)
        .await
        .map_err(from_conversation)?;
    if let Some(project_id) = conversation.project_id {
        let ready: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.projects WHERE id=$1 AND user_id=$2 AND environment_id=$3 AND state='ready')")
            .bind(project_id).bind(actor.user_id).bind(env).fetch_one(&mut **tx).await.map_err(unavailable)?;
        if !ready {
            return Err(AttachmentError::Conflict);
        }
    }
    Ok(())
}

/// Attachment IDs of a run's user message, in message order.
pub(crate) async fn user_attachment_ids(
    connection: &mut PgConnection,
    run_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM factory_identity.conversation_attachments WHERE run_id=$1 AND source='user' ORDER BY ordinal")
        .bind(run_id).fetch_all(connection).await
}

/// Atomically attach staged uploads to the user message of a new turn.
pub(crate) async fn claim_staged(
    connection: &mut PgConnection,
    user_id: Uuid,
    conversation_id: Uuid,
    environment_id: Uuid,
    run_id: Uuid,
    message_id: Uuid,
    ids: &[Uuid],
) -> Result<(), ConversationError> {
    if ids.is_empty() {
        return Ok(());
    }
    let ordinals: Vec<i16> = (0..ids.len() as i16).collect();
    let claimed = sqlx::query("UPDATE factory_identity.conversation_attachments a SET state='attached',run_id=$1,message_id=$2,ordinal=u.ordinal FROM unnest($3::uuid[],$4::smallint[]) AS u(id,ordinal) WHERE a.id=u.id AND a.user_id=$5 AND a.conversation_id=$6 AND a.environment_id=$7 AND a.source='user' AND a.state='staged'")
        .bind(run_id).bind(message_id).bind(ids).bind(&ordinals).bind(user_id).bind(conversation_id).bind(environment_id)
        .execute(connection).await.map_err(|_| ConversationError::Unavailable)?;
    if claimed.rows_affected() != ids.len() as u64 {
        return Err(ConversationError::Conflict);
    }
    Ok(())
}

/// Attachments of the given messages, in message order and then file order.
pub(crate) async fn for_messages(
    connection: &mut PgConnection,
    messages: &[Uuid],
) -> Result<Vec<Attachment>, AttachmentError> {
    let rows = sqlx::query(&format!("SELECT {COLUMNS} FROM factory_identity.conversation_attachments WHERE message_id = ANY($1) ORDER BY message_id,ordinal"))
        .bind(messages).fetch_all(connection).await.map_err(unavailable)?;
    let mut found = rows.iter().map(attachment).collect::<Result<Vec<_>, _>>()?;
    found.sort_by_key(|a| {
        messages
            .iter()
            .position(|id| Some(*id) == a.message_id)
            .unwrap_or(usize::MAX)
    });
    Ok(found)
}

#[async_trait::async_trait]
impl AttachmentStore for PgAttachmentStore {
    async fn reserve(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        new: NewAttachment,
    ) -> Result<Attachment, AttachmentError> {
        if new.id.is_nil() || new.conversation_id.is_nil() {
            return Err(AttachmentError::Invalid);
        }
        validate_name(&new.name)?;
        with_connection(&self.pool, || AttachmentError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await.map_err(from_conversation)?;
                usable_conversation(&mut tx, actor, env, new.conversation_id).await?;
                let row = sqlx::query(&format!("INSERT INTO factory_identity.conversation_attachments(id,user_id,conversation_id,source,state,name,media_type,kind,size_bytes,environment_id,environment_generation) VALUES($1,$2,$3,'user','uploading',$4,'application/octet-stream','file',0,$5,$6) RETURNING {COLUMNS}"))
                    .bind(new.id).bind(actor.user_id).bind(new.conversation_id).bind(&new.name).bind(env).bind(generation)
                    .fetch_one(&mut *tx).await.map_err(|error| {
                        if error.as_database_error().and_then(|e| e.constraint()) == Some("conversation_attachments_pkey") {
                            AttachmentError::Conflict
                        } else { unavailable(error) }
                    })?;
                let result = attachment(&row)?;
                tx.commit().await.map_err(unavailable)?;
                Ok(result)
            })
        })
        .await
    }

    async fn mark_staged(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        id: Uuid,
        file: StoredFile,
    ) -> Result<Attachment, AttachmentError> {
        let size = valid_file(&file)?;
        with_connection(&self.pool, || AttachmentError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await.map_err(from_conversation)?;
                let row = sqlx::query(&format!("UPDATE factory_identity.conversation_attachments SET state='staged',media_type=$4,kind=$5,size_bytes=$6,sha256=$7 WHERE id=$1 AND user_id=$2 AND environment_id=$3 AND source='user' AND state='uploading' RETURNING {COLUMNS}"))
                    .bind(id).bind(actor.user_id).bind(env).bind(&file.media_type).bind(file.kind.as_str()).bind(size).bind(&file.sha256)
                    .fetch_optional(&mut *tx).await.map_err(unavailable)?;
                let row = match row {
                    Some(row) => row,
                    None => {
                        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM factory_identity.conversation_attachments WHERE id=$1 AND user_id=$2 AND environment_id=$3)")
                            .bind(id).bind(actor.user_id).bind(env).fetch_one(&mut *tx).await.map_err(unavailable)?;
                        return Err(if exists { AttachmentError::Conflict } else { AttachmentError::NotFound });
                    }
                };
                let result = attachment(&row)?;
                tx.commit().await.map_err(unavailable)?;
                Ok(result)
            })
        })
        .await
    }

    async fn remove_staged(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        id: Uuid,
    ) -> Result<Attachment, AttachmentError> {
        with_connection(&self.pool, || AttachmentError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await.map_err(from_conversation)?;
                let row = sqlx::query(&format!("SELECT {COLUMNS} FROM factory_identity.conversation_attachments WHERE id=$1 AND user_id=$2 AND environment_id=$3 FOR UPDATE"))
                    .bind(id).bind(actor.user_id).bind(env).fetch_optional(&mut *tx).await.map_err(unavailable)?
                    .ok_or(AttachmentError::NotFound)?;
                let existing = attachment(&row)?;
                if existing.state != AttachmentState::Staged {
                    return Err(AttachmentError::Conflict);
                }
                sqlx::query("DELETE FROM factory_identity.conversation_attachments WHERE id=$1")
                    .bind(id).execute(&mut *tx).await.map_err(unavailable)?;
                tx.commit().await.map_err(unavailable)?;
                Ok(existing)
            })
        })
        .await
    }

    async fn expire_staged(
        &self,
        actor: EnvironmentActor,
        before: DateTime<Utc>,
    ) -> Result<Vec<Attachment>, AttachmentError> {
        with_connection(&self.pool, || AttachmentError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize_actor(&mut tx, actor).await.map_err(from_conversation)?;
                let rows = sqlx::query(&format!("DELETE FROM factory_identity.conversation_attachments WHERE id IN (SELECT id FROM factory_identity.conversation_attachments WHERE user_id=$1 AND state IN ('staged','uploading') AND created_at < $2 ORDER BY created_at,id LIMIT $3 FOR UPDATE) RETURNING {COLUMNS}"))
                    .bind(actor.user_id).bind(before).bind(EXPIRE_BATCH).fetch_all(&mut *tx).await.map_err(unavailable)?;
                let expired = rows.iter().map(attachment).collect::<Result<Vec<_>, _>>()?;
                tx.commit().await.map_err(unavailable)?;
                Ok(expired)
            })
        })
        .await
    }

    async fn get(&self, actor: EnvironmentActor, id: Uuid) -> Result<Attachment, AttachmentError> {
        with_connection(&self.pool, || AttachmentError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize_actor(&mut tx, actor).await.map_err(from_conversation)?;
                let row = sqlx::query(&format!("SELECT {COLUMNS} FROM factory_identity.conversation_attachments WHERE id=$1 AND user_id=$2"))
                    .bind(id).bind(actor.user_id).fetch_optional(&mut *tx).await.map_err(unavailable)?
                    .ok_or(AttachmentError::NotFound)?;
                let result = attachment(&row)?;
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
        conversation_id: Uuid,
    ) -> Result<Vec<Attachment>, AttachmentError> {
        with_connection(&self.pool, || AttachmentError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await.map_err(from_conversation)?;
                owned(&mut tx, actor.user_id, conversation_id).await.map_err(from_conversation)?;
                let rows = sqlx::query(&format!("SELECT {COLUMNS} FROM factory_identity.conversation_attachments WHERE conversation_id=$1 AND user_id=$2 ORDER BY created_at,id LIMIT $3"))
                    .bind(conversation_id).bind(actor.user_id).bind(LIST_LIMIT).fetch_all(&mut *tx).await.map_err(unavailable)?;
                let result = rows.iter().map(attachment).collect::<Result<Vec<_>, _>>()?;
                tx.commit().await.map_err(unavailable)?;
                Ok(result)
            })
        })
        .await
    }

    async fn record_agent_attachments(
        &self,
        actor: EnvironmentActor,
        env: Uuid,
        generation: i64,
        run_id: Uuid,
        message_id: Uuid,
        attachments: Vec<AgentAttachment>,
    ) -> Result<Vec<Attachment>, AttachmentError> {
        if attachments.len() > MAX_ATTACHMENTS_PER_MESSAGE {
            return Err(AttachmentError::TooMany);
        }
        let ids: Vec<Uuid> = attachments.iter().map(|a| a.id).collect();
        if ids.iter().any(Uuid::is_nil) {
            return Err(AttachmentError::Invalid);
        }
        validate_attachment_ids(&ids)?;
        for item in &attachments {
            validate_name(&item.name)?;
            if let AgentCapture::Captured(file) = &item.capture {
                valid_file(file)?;
            }
        }
        with_connection(&self.pool, || AttachmentError::Unavailable, |connection| {
            Box::pin(async move {
                let mut tx = connection.begin().await.map_err(unavailable)?;
                authorize(&mut tx, actor, env, generation).await.map_err(from_conversation)?;
                let run = owned_run(&mut tx, actor.user_id, env, generation, run_id).await.map_err(from_conversation)?;
                let role: Option<String> = sqlx::query_scalar("SELECT role FROM factory_identity.conversation_messages WHERE id=$1 AND run_id=$2")
                    .bind(message_id).bind(run_id).fetch_optional(&mut *tx).await.map_err(unavailable)?;
                if role.as_deref() != Some("assistant") {
                    return Err(AttachmentError::NotFound);
                }
                let existing = sqlx::query(&format!("SELECT {COLUMNS} FROM factory_identity.conversation_attachments WHERE message_id=$1 ORDER BY ordinal"))
                    .bind(message_id).fetch_all(&mut *tx).await.map_err(unavailable)?;
                if !existing.is_empty() {
                    let found = existing.iter().map(attachment).collect::<Result<Vec<_>, _>>()?;
                    let same = found.len() == ids.len()
                        && found.iter().zip(&attachments).all(|(old, new)| {
                            old.id == new.id && old.source == AttachmentSource::Agent && old.name == new.name
                        });
                    if !same {
                        return Err(AttachmentError::Conflict);
                    }
                    tx.commit().await.map_err(unavailable)?;
                    return Ok(found);
                }
                let mut result = Vec::with_capacity(attachments.len());
                for (ordinal, item) in attachments.iter().enumerate() {
                    let ordinal = ordinal as i16;
                    let insert = format!("INSERT INTO factory_identity.conversation_attachments(id,user_id,conversation_id,run_id,message_id,source,state,name,media_type,kind,size_bytes,sha256,ordinal,environment_id,environment_generation) VALUES($1,$2,$3,$4,$5,'agent','attached',$6,$7,$8,$9,$10,$11,$12,$13) RETURNING {COLUMNS}");
                    let missing = format!("INSERT INTO factory_identity.conversation_attachments(id,user_id,conversation_id,run_id,message_id,source,state,name,media_type,kind,size_bytes,unavailable_code,ordinal,environment_id,environment_generation) VALUES($1,$2,$3,$4,$5,'agent','unavailable',$6,'application/octet-stream','file',0,$7,$8,$9,$10) RETURNING {COLUMNS}");
                    let query = match &item.capture {
                        AgentCapture::Captured(file) => sqlx::query(&insert)
                            .bind(item.id).bind(actor.user_id).bind(run.conversation_id).bind(run_id).bind(message_id).bind(&item.name)
                            .bind(&file.media_type).bind(file.kind.as_str()).bind(valid_file(file)?).bind(&file.sha256).bind(ordinal).bind(env).bind(generation),
                        AgentCapture::Unavailable => sqlx::query(&missing)
                            .bind(item.id).bind(actor.user_id).bind(run.conversation_id).bind(run_id).bind(message_id).bind(&item.name)
                            .bind(UNAVAILABLE_CODE).bind(ordinal).bind(env).bind(generation),
                    };
                    let row = query.fetch_one(&mut *tx).await.map_err(|error| {
                        if error.as_database_error().is_some_and(|e| e.is_unique_violation()) {
                            AttachmentError::Conflict
                        } else { unavailable(error) }
                    })?;
                    result.push(attachment(&row)?);
                }
                tx.commit().await.map_err(unavailable)?;
                Ok(result)
            })
        })
        .await
    }
}
