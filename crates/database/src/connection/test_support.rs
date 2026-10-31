use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions, PgSslMode},
};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinHandle,
};

pub(crate) fn options(listener: &TcpListener) -> PgConnectOptions {
    PgConnectOptions::new_without_pgpass()
        .host("127.0.0.1")
        .port(listener.local_addr().unwrap().port())
        .username("fixture")
        .database("fixture")
        .ssl_mode(PgSslMode::Disable)
}

pub(crate) fn pool(listener: &TcpListener) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(2))
        .test_before_acquire(false)
        .connect_lazy_with(options(listener))
}

pub(crate) async fn accept(listener: &TcpListener) -> TcpStream {
    let (mut socket, _) = listener.accept().await.unwrap();
    let length = socket.read_u32().await.unwrap() as usize;
    assert!((4..=4096).contains(&length));
    let mut startup = vec![0; length - 4];
    socket.read_exact(&mut startup).await.unwrap();
    // PostgreSQL AuthenticationOk followed by ReadyForQuery (idle).
    socket
        .write_all(b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I")
        .await
        .unwrap();
    socket
}

pub(crate) async fn wait_for_query(socket: &mut TcpStream, query: &[u8]) {
    let mut request = Vec::new();
    let mut buffer = [0; 512];
    while !request.windows(query.len()).any(|part| part == query) {
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0, "the operation must reach its SQL query");
        request.extend_from_slice(&buffer[..count]);
        assert!(request.len() <= 4096);
    }
}

pub(crate) async fn wait_for_disconnect(socket: &mut TcpStream) {
    let mut buffer = [0; 512];
    while socket.read(&mut buffer).await.unwrap() != 0 {}
}

pub(crate) async fn stalled_pool(
    query: &'static [u8],
    transaction: bool,
) -> (PgPool, oneshot::Receiver<()>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pool = pool(&listener);
    let (reached, ready) = oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        if transaction {
            wait_for_query(&mut socket, b"BEGIN").await;
            // CommandComplete (BEGIN), then ReadyForQuery (in transaction).
            socket
                .write_all(b"C\0\0\0\x0aBEGIN\0Z\0\0\0\x05T")
                .await
                .unwrap();
        }
        wait_for_query(&mut socket, query).await;
        reached.send(()).unwrap();
        wait_for_disconnect(&mut socket).await;
        let mut replacement = accept(&listener).await;
        wait_for_disconnect(&mut replacement).await;
    });
    (pool, ready, peer)
}

pub(crate) async fn finish_replacement(pool: PgPool, peer: JoinHandle<()>) {
    tokio::time::timeout(Duration::from_secs(2), async {
        // With one permit, this succeeds only if interruption discarded the old
        // socket and released capacity, rather than leaving a pool-return ping.
        let replacement = pool.acquire().await.unwrap();
        replacement.close().await.unwrap();
        peer.await.unwrap();
        pool.close().await;
    })
    .await
    .expect("the one-connection pool must accept a fresh connection");
}
