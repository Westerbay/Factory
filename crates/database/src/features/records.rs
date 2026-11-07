//! Mapping and updates for the ordered records belonging to a single feature.
use super::*;
use serde::de::DeserializeOwned;

async fn load_records<T: DeserializeOwned + Send + Unpin + 'static>(
    tx: &mut Transaction<'_, Postgres>,
    select: &str,
    id: Uuid,
) -> Result<Vec<T>, FeatureError> {
    let records = sqlx::query_scalar::<_, Json<T>>(select)
        .bind(id)
        .fetch_all(&mut **tx)
        .await
        .map_err(unavailable)?;
    Ok(records.into_iter().map(|Json(record)| record).collect())
}

pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    row: &PgRow,
) -> Result<FeatureState, FeatureError> {
    let feature = feature(row)?;
    let id = feature.id;
    let rounds: i64 = row
        .try_get("additional_review_rounds")
        .map_err(unavailable)?;
    Ok(FeatureState {
        feature,
        revisions: load_records(tx, "SELECT record FROM factory_identity.feature_revisions WHERE feature_id=$1 ORDER BY position", id).await?,
        scope: row.try_get::<Option<Json<RevisionKey>>, _>("scope").map_err(unavailable)?.map(|Json(key)| key),
        architecture: row.try_get::<Option<Json<RevisionKey>>, _>("architecture").map_err(unavailable)?.map(|Json(key)| key),
        attempts: load_records(tx, "SELECT record FROM factory_identity.feature_attempts WHERE feature_id=$1 ORDER BY position", id).await?,
        reviews: load_records(tx, "SELECT record FROM factory_identity.feature_reviews WHERE feature_id=$1 ORDER BY position", id).await?,
        current_review: row.try_get("current_review").map_err(unavailable)?,
        additional_review_rounds: rounds.try_into().map_err(unavailable)?,
        deliveries: load_records(tx, "SELECT record FROM factory_identity.feature_deliveries WHERE feature_id=$1 ORDER BY position", id).await?,
    })
}

fn changed<'a, T: PartialEq>(
    previous: &'a [T],
    current: &'a [T],
) -> impl Iterator<Item = (usize, &'a T)> {
    current
        .iter()
        .enumerate()
        .filter(move |(position, item)| previous.get(*position) != Some(*item))
}

pub(super) async fn update(
    tx: &mut Transaction<'_, Postgres>,
    previous: &FeatureState,
    state: &FeatureState,
) -> Result<(), FeatureError> {
    let id = state.feature.id;
    sqlx::query(
        "UPDATE factory_identity.features SET
         stage=$2, scope=$3, architecture=$4, current_review=$5, additional_review_rounds=$6 WHERE id=$1",
    )
    .bind(id)
    .bind(stage_name(state.feature.stage))
    .bind(state.scope.as_ref().map(Json))
    .bind(state.architecture.as_ref().map(Json))
    .bind(state.current_review)
    .bind(i64::from(state.additional_review_rounds))
    .execute(&mut **tx)
    .await
    .map_err(unavailable)?;
    for (position, revision) in changed(&previous.revisions, &state.revisions) {
        sqlx::query(
            "INSERT INTO factory_identity.feature_revisions(feature_id,id,position,record) VALUES($1,$2,$3,$4)
             ON CONFLICT (feature_id,id) DO UPDATE SET record=EXCLUDED.record",
        )
        .bind(id)
        .bind(revision.key.id)
        .bind(i64::try_from(position).map_err(unavailable)?)
        .bind(Json(revision))
        .execute(&mut **tx)
        .await
        .map_err(unavailable)?;
    }
    for (position, attempt) in changed(&previous.attempts, &state.attempts) {
        sqlx::query(
            "INSERT INTO factory_identity.feature_attempts(feature_id,id,position,reserved,record) VALUES($1,$2,$3,$4,$5)
             ON CONFLICT (feature_id,id) DO UPDATE SET reserved=EXCLUDED.reserved,record=EXCLUDED.record",
        )
        .bind(id)
        .bind(attempt.id())
        .bind(i64::try_from(position).map_err(unavailable)?)
        .bind(attempt.status.holds_reservation())
        .bind(Json(attempt))
        .execute(&mut **tx)
        .await
        .map_err(unavailable)?;
    }
    for (position, round) in changed(&previous.reviews, &state.reviews) {
        sqlx::query(
            "INSERT INTO factory_identity.feature_reviews(feature_id,id,position,record) VALUES($1,$2,$3,$4)
             ON CONFLICT (feature_id,id) DO UPDATE SET record=EXCLUDED.record",
        )
        .bind(id)
        .bind(round.snapshot.id)
        .bind(i64::try_from(position).map_err(unavailable)?)
        .bind(Json(round))
        .execute(&mut **tx)
        .await
        .map_err(unavailable)?;
    }
    for (position, delivery) in changed(&previous.deliveries, &state.deliveries) {
        sqlx::query(
            "INSERT INTO factory_identity.feature_deliveries(feature_id,id,position,unsettled,record) VALUES($1,$2,$3,$4,$5)
             ON CONFLICT (feature_id,id) DO UPDATE SET unsettled=EXCLUDED.unsettled,record=EXCLUDED.record",
        )
        .bind(id)
        .bind(delivery.request_id)
        .bind(i64::try_from(position).map_err(unavailable)?)
        .bind(matches!(delivery.status, DeliveryStatus::Claimed | DeliveryStatus::Uncertain))
        .bind(Json(delivery))
        .execute(&mut **tx)
        .await
        .map_err(unavailable)?;
    }
    Ok(())
}
