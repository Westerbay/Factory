use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    routing::get,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{Notify, mpsc, oneshot};

const TEST_INTERVAL: Duration = Duration::from_millis(10);
const TEST_DEADLINE: Duration = Duration::from_secs(3);

#[derive(Default)]
struct HostState {
    failed: AtomicBool,
    ready: AtomicBool,
    closed: AtomicUsize,
    draining: Notify,
    drained: AtomicBool,
}

struct TestHost {
    state: Arc<HostState>,
    drain: Option<oneshot::Receiver<bool>>,
}

#[async_trait::async_trait]
impl RecoveryHost for TestHost {
    fn is_failed(&self) -> bool {
        self.state.failed.load(Ordering::SeqCst)
    }

    fn is_ready(&self) -> bool {
        !self.is_failed() && self.state.ready.load(Ordering::SeqCst)
    }

    fn close(&self) {
        self.state.failed.store(true, Ordering::SeqCst);
        self.state.closed.fetch_add(1, Ordering::SeqCst);
    }

    async fn drain(&mut self) -> bool {
        assert!(self.is_failed(), "permissions must close before draining");
        self.state.draining.notify_one();
        let clean = match self.drain.take() {
            Some(result) => result.await.unwrap(),
            None => true,
        };
        self.state.drained.store(true, Ordering::SeqCst);
        clean
    }
}

fn epoch(name: &'static str) -> (HostRouter<TestHost>, Arc<HostState>) {
    let state = Arc::new(HostState::default());
    state.ready.store(true, Ordering::SeqCst);
    let handler_state = state.clone();
    let router = Router::new().route(
        "/runtime",
        get(move || {
            let state = handler_state.clone();
            async move {
                let status =
                    if state.failed.load(Ordering::SeqCst) || !state.ready.load(Ordering::SeqCst) {
                        StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        StatusCode::OK
                    };
                (status, name)
            }
        }),
    );
    (
        HostRouter {
            router,
            host: TestHost {
                state: state.clone(),
                drain: None,
            },
        },
        state,
    )
}

async fn response(router: &Router) -> (StatusCode, String) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/runtime")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 128).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

async fn current_response(current: &watch::Receiver<Router>) -> (StatusCode, String) {
    let router = current.borrow().clone();
    response(&router).await
}

#[tokio::test]
async fn recovery_waits_for_old_jobs_and_retries_one_acquisition_at_a_time() {
    let (mut old, old_state) = epoch("old");
    old_state.failed.store(true, Ordering::SeqCst);
    let old_router = old.router.clone();
    let (release_drain, drain) = oneshot::channel();
    old.host.drain = Some(drain);
    let (next, next_state) = epoch("replacement");
    let (published, mut current) = watch::channel(old_router.clone());
    let (stop, stopped) = watch::channel(false);
    let (attempted, mut attempts) = mpsc::unbounded_channel();
    let mut replies = Vec::new();
    let mut pending = Vec::new();
    for _ in 0..4 {
        let (reply, result) = oneshot::channel();
        replies.push(reply);
        pending.push(result);
    }
    let mut pending = pending.into_iter().enumerate();
    let worker = tokio::spawn(supervise(
        Some(old),
        published,
        move || {
            let (index, result) = pending.next().expect("no extra acquisition");
            attempted.send(index).unwrap();
            async move { result.await.unwrap() }
        },
        stopped,
        TEST_INTERVAL,
    ));

    tokio::time::timeout(TEST_DEADLINE, old_state.draining.notified())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(TEST_INTERVAL * 3, attempts.recv())
            .await
            .is_err(),
        "ownership cannot be reacquired while an old job is still draining"
    );
    assert_eq!(old_state.closed.load(Ordering::SeqCst), 1);
    release_drain.send(true).unwrap();

    let mut replies = replies.into_iter();
    for (index, error) in [
        EnvironmentError::Conflict,
        EnvironmentError::Unavailable,
        EnvironmentError::Timeout,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            tokio::time::timeout(TEST_DEADLINE, attempts.recv())
                .await
                .unwrap(),
            Some(index)
        );
        assert!(old_state.drained.load(Ordering::SeqCst));
        assert!(
            tokio::time::timeout(TEST_INTERVAL * 3, attempts.recv())
                .await
                .is_err(),
            "a pending acquisition must not overlap a retry"
        );
        assert!(!current.has_changed().unwrap());
        assert_eq!(
            response(&old_router).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        replies.next().unwrap().send(Err(error)).ok().unwrap();
    }
    assert_eq!(
        tokio::time::timeout(TEST_DEADLINE, attempts.recv())
            .await
            .unwrap(),
        Some(3)
    );
    replies.next().unwrap().send(Ok(next)).ok().unwrap();
    tokio::time::timeout(TEST_DEADLINE, current.changed())
        .await
        .unwrap()
        .unwrap();
    let replacement = current.borrow().clone();
    assert_eq!(
        response(&replacement).await,
        (StatusCode::OK, "replacement".into())
    );
    assert_eq!(
        response(&old_router).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(!next_state.failed.load(Ordering::SeqCst));

    stop.send_replace(true);
    tokio::time::timeout(TEST_DEADLINE, worker)
        .await
        .unwrap()
        .unwrap();
    assert!(next_state.failed.load(Ordering::SeqCst));
    assert!(next_state.drained.load(Ordering::SeqCst));
}

#[tokio::test]
async fn the_same_router_dispatches_new_requests_to_the_replacement_epoch() {
    let (old, old_state) = epoch("old");
    let old_router = old.router.clone();
    let (next, next_state) = epoch("replacement");
    let mut next = Some(next);
    let (stop, stopped) = watch::channel(false);
    let (router, worker) = recovering_router(
        Router::new(),
        Some(old),
        move || std::future::ready(Ok(next.take().expect("only one replacement"))),
        stopped,
    );
    assert_eq!(response(&router).await, (StatusCode::OK, "old".into()));
    old_state.failed.store(true, Ordering::SeqCst);
    tokio::time::timeout(TEST_DEADLINE, async {
        loop {
            if response(&router).await == (StatusCode::OK, "replacement".into()) {
                break;
            }
            tokio::time::sleep(TEST_INTERVAL).await;
        }
    })
    .await
    .expect("existing listener/router must observe the replacement");
    assert_eq!(
        response(&old_router).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(old_state.drained.load(Ordering::SeqCst));
    stop.send_replace(true);
    tokio::time::timeout(TEST_DEADLINE, worker)
        .await
        .unwrap()
        .unwrap();
    assert!(next_state.drained.load(Ordering::SeqCst));
    assert_eq!(response(&router).await.0, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn hosts_that_fail_before_readiness_keep_increasing_the_retry_delay() {
    let (published, _current) = watch::channel(Router::new());
    let (stop, stopped) = watch::channel(false);
    let (attempted, mut attempts) = mpsc::unbounded_channel();
    let interval = Duration::from_millis(20);
    let mut count = 0;
    let worker = tokio::spawn(supervise(
        None,
        published,
        move || {
            count += 1;
            assert!(count <= 4, "unexpected extra acquisition");
            let (candidate, state) = epoch("candidate");
            if count < 4 {
                state.ready.store(false, Ordering::SeqCst);
                state.failed.store(true, Ordering::SeqCst);
            }
            attempted.send(tokio::time::Instant::now()).unwrap();
            std::future::ready(Ok(candidate))
        },
        stopped,
        interval,
    ));
    let mut previous = tokio::time::timeout(TEST_DEADLINE, attempts.recv())
        .await
        .unwrap()
        .unwrap();
    for multiplier in [2, 4, 8] {
        let attempted_at = tokio::time::timeout(TEST_DEADLINE, attempts.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            attempted_at.duration_since(previous)
                >= interval * multiplier - Duration::from_millis(2),
            "constructing an unhealthy host must not reset the backoff"
        );
        previous = attempted_at;
    }
    stop.send_replace(true);
    tokio::time::timeout(TEST_DEADLINE, worker)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn permanent_drain_failure_never_reacquires_or_publishes() {
    let (mut old, old_state) = epoch("old");
    old_state.failed.store(true, Ordering::SeqCst);
    let (finish, drain) = oneshot::channel();
    old.host.drain = Some(drain);
    let (published, current) = watch::channel(old.router.clone());
    let (_stop, stopped) = watch::channel(false);
    let worker = tokio::spawn(supervise(
        Some(old),
        published,
        || async { panic!("a classified worker panic or permanent failure cannot retry") },
        stopped,
        TEST_INTERVAL,
    ));
    tokio::time::timeout(TEST_DEADLINE, old_state.draining.notified())
        .await
        .unwrap();
    finish.send(false).unwrap();
    tokio::time::timeout(TEST_DEADLINE, worker)
        .await
        .unwrap()
        .unwrap();
    assert!(old_state.failed.load(Ordering::SeqCst));
    assert!(old_state.drained.load(Ordering::SeqCst));
    assert_eq!(
        current_response(&current).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn invalid_configuration_and_foreign_ownership_do_not_retry() {
    for error in [
        EnvironmentError::InvalidConfiguration,
        EnvironmentError::OwnershipConflict,
    ] {
        let (fallback, state) = epoch("fallback");
        state.failed.store(true, Ordering::SeqCst);
        let (published, current) = watch::channel(fallback.router);
        let (_stop, stopped) = watch::channel(false);
        let attempts = Arc::new(AtomicUsize::new(0));
        let calls = attempts.clone();
        tokio::time::timeout(
            TEST_DEADLINE,
            supervise::<TestHost, _, _>(
                None,
                published,
                move || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Err(error))
                },
                stopped,
                TEST_INTERVAL,
            ),
        )
        .await
        .unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(current_response(&current).await.1, "fallback");
    }
}

#[tokio::test]
async fn shutdown_during_backoff_prevents_further_acquisition() {
    let (fallback, _) = epoch("fallback");
    let (published, current) = watch::channel(fallback.router);
    let (stop, stopped) = watch::channel(false);
    let (attempted, mut attempts) = mpsc::unbounded_channel();
    let worker = tokio::spawn(supervise::<TestHost, _, _>(
        None,
        published,
        move || {
            attempted.send(()).unwrap();
            std::future::ready(Err(EnvironmentError::Unavailable))
        },
        stopped,
        TEST_INTERVAL,
    ));
    tokio::time::timeout(TEST_DEADLINE, attempts.recv())
        .await
        .unwrap()
        .unwrap();
    stop.send_replace(true);
    tokio::time::timeout(TEST_DEADLINE, worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(attempts.recv().await, None);
    assert_eq!(current_response(&current).await.1, "fallback");
}

struct Cancelled(Arc<AtomicBool>);

impl Drop for Cancelled {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn shutdown_cancels_pending_acquisition_without_publishing() {
    let (fallback, _) = epoch("fallback");
    let (published, current) = watch::channel(fallback.router);
    let (stop, stopped) = watch::channel(false);
    let entered = Arc::new(Notify::new());
    let cancelled = Arc::new(AtomicBool::new(false));
    let creating = entered.clone();
    let dropped = cancelled.clone();
    let worker = tokio::spawn(supervise::<TestHost, _, _>(
        None,
        published,
        move || {
            let creating = creating.clone();
            let dropped = dropped.clone();
            async move {
                let _cancelled = Cancelled(dropped);
                creating.notify_one();
                std::future::pending().await
            }
        },
        stopped,
        TEST_INTERVAL,
    ));
    tokio::time::timeout(TEST_DEADLINE, entered.notified())
        .await
        .unwrap();
    stop.send_replace(true);
    tokio::time::timeout(TEST_DEADLINE, worker)
        .await
        .unwrap()
        .unwrap();
    assert!(cancelled.load(Ordering::SeqCst));
    assert_eq!(current_response(&current).await.1, "fallback");
}

#[tokio::test]
async fn shutdown_racing_a_completed_candidate_drains_it_without_publishing() {
    let (fallback, _) = epoch("fallback");
    let (published, current) = watch::channel(fallback.router);
    let (stop, stopped) = watch::channel(false);
    let (mut next, next_state) = epoch("candidate");
    let (finish, drain) = oneshot::channel();
    next.host.drain = Some(drain);
    let mut next = Some(next);
    let worker = tokio::spawn(supervise(
        None,
        published,
        move || {
            let candidate = next.take().expect("only one candidate");
            let stop = stop.clone();
            async move {
                // Stop becomes ready in the same poll that completes acquisition.
                stop.send_replace(true);
                Ok(candidate)
            }
        },
        stopped,
        TEST_INTERVAL,
    ));
    tokio::time::timeout(TEST_DEADLINE, next_state.draining.notified())
        .await
        .unwrap();
    assert!(next_state.failed.load(Ordering::SeqCst));
    assert!(
        !worker.is_finished(),
        "shutdown must wait for the candidate's jobs"
    );
    assert!(!current.has_changed().unwrap());
    assert_eq!(current_response(&current).await.1, "fallback");
    finish.send(true).unwrap();
    tokio::time::timeout(TEST_DEADLINE, worker)
        .await
        .unwrap()
        .unwrap();
    assert!(next_state.drained.load(Ordering::SeqCst));
    assert_eq!(next_state.closed.load(Ordering::SeqCst), 1);
}
