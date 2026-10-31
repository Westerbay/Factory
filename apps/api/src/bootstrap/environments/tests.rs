use super::*;

#[tokio::test]
async fn nested_job_panic_closes_the_host_and_disables_automatic_recovery() {
    let availability = EnvironmentAvailability::default();
    let fatal = AtomicBool::new(false);
    let job: JoinHandle<Result<(), EnvironmentError>> =
        tokio::spawn(async { panic!("fixture worker panic") });
    observe_job(job.await, &availability, &fatal);
    assert!(availability.is_failed());
    assert!(fatal.load(Ordering::SeqCst));
}

#[test]
fn worker_storage_failure_can_recover_but_resource_ownership_failure_cannot() {
    for (error, permanent) in [
        (EnvironmentError::Unavailable, false),
        (EnvironmentError::Timeout, false),
        (EnvironmentError::OwnershipConflict, true),
        (EnvironmentError::InvalidConfiguration, true),
    ] {
        let availability = EnvironmentAvailability::default();
        let fatal = AtomicBool::new(false);
        observe_job(Ok(Err(error)), &availability, &fatal);
        assert!(availability.is_failed());
        assert_eq!(fatal.load(Ordering::SeqCst), permanent);
    }
}
