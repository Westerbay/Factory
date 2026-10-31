use super::test_support::{accept, finish_replacement, pool, wait_for_disconnect, wait_for_query};
use super::*;
use sqlx::{Connection, Executor};
use tokio::{io::AsyncWriteExt, net::TcpListener, sync::oneshot};

async fn stalled_return(cancel: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pool = pool(&listener);
    let (reached, ready) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        wait_for_query(&mut socket, b"SELECT 1").await;
        // Confirm the SQL result before withholding the pool-return ping reply.
        socket
            .write_all(b"C\0\0\0\x0dSELECT 1\0Z\0\0\0\x05I")
            .await
            .unwrap();
        wait_for_query(&mut socket, b"S\0\0\0\x04").await;
        reached.send(()).unwrap();
        wait_for_disconnect(&mut socket).await;
        let mut replacement = accept(&listener).await;
        wait_for_disconnect(&mut replacement).await;
    });
    let operation_pool = pool.clone();
    let operation = tokio::spawn(async move {
        with_connection(
            &operation_pool,
            || "unavailable",
            |connection| {
                Box::pin(async move {
                    let result = connection
                        .execute("SELECT 1")
                        .await
                        .map_err(|_| "SQL failed")?;
                    Ok(result.rows_affected())
                })
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .expect("confirmed SQL must reach pool recycling")
        .unwrap();
    if cancel {
        operation.abort();
        assert!(operation.await.is_err_and(|error| error.is_cancelled()));
    } else {
        let result = tokio::time::timeout(Duration::from_secs(12), operation)
            .await
            .expect("a stalled pool return must be bounded")
            .unwrap();
        assert_eq!(
            result,
            Ok(1),
            "recycling must preserve the confirmed SQL result"
        );
    }
    finish_replacement(pool, peer).await;
}

#[tokio::test]
async fn stalled_pool_return_keeps_the_confirmed_result_and_releases_capacity() {
    stalled_return(false).await;
}

#[tokio::test]
async fn cancelling_pool_return_releases_capacity() {
    stalled_return(true).await;
}

#[tokio::test]
async fn unacknowledged_commit_is_unavailable_and_is_never_replayed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pool = pool(&listener);
    let (reached, ready) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        wait_for_query(&mut socket, b"BEGIN").await;
        socket
            .write_all(b"C\0\0\0\x0aBEGIN\0Z\0\0\0\x05T")
            .await
            .unwrap();
        wait_for_query(&mut socket, b"UPDATE fixture SET value=1").await;
        socket
            .write_all(b"C\0\0\0\x0dUPDATE 1\0Z\0\0\0\x05T")
            .await
            .unwrap();
        wait_for_query(&mut socket, b"COMMIT").await;
        reached.send(()).unwrap();
        wait_for_disconnect(&mut socket).await;
        let mut replacement = accept(&listener).await;
        // The next connection is acquired only by the test. Any replay would
        // send SQL here and prevent the checked-out replacement from opening.
        wait_for_disconnect(&mut replacement).await;
    });
    let operation_pool = pool.clone();
    let operation = tokio::spawn(async move {
        with_connection(
            &operation_pool,
            || "unavailable",
            |connection| {
                Box::pin(async move {
                    let mut transaction = connection.begin().await.map_err(|_| "SQL failed")?;
                    transaction
                        .execute("UPDATE fixture SET value=1")
                        .await
                        .map_err(|_| "SQL failed")?;
                    transaction.commit().await.map_err(|_| "SQL failed")?;
                    Ok(())
                })
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), ready)
        .await
        .expect("the write must reach COMMIT")
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(12), operation)
        .await
        .expect("an unacknowledged commit must be bounded")
        .unwrap();
    assert_eq!(result, Err("unavailable"));
    finish_replacement(pool, peer).await;
}
