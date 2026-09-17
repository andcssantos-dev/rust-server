use std::{
    collections::HashSet,
    sync::{Arc, Mutex, MutexGuard},
};

use aurenfall_domain::{PreparedQuadrantArtifact, QuadrantPreparationIdentity};
use thiserror::Error;
use tokio::{
    sync::mpsc,
    task::{JoinError, JoinHandle, JoinSet},
};

pub const MAX_PREPARATION_QUEUE_CAPACITY: usize = 65_536;
pub const MAX_PREPARATION_CONCURRENCY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparationWorkerSettings {
    request_queue_capacity: usize,
    result_queue_capacity: usize,
    max_in_flight: usize,
}

impl PreparationWorkerSettings {
    pub fn new(
        request_queue_capacity: usize,
        result_queue_capacity: usize,
        max_in_flight: usize,
    ) -> Result<Self, PreparationWorkerSettingsError> {
        validate_capacity(
            "request_queue_capacity",
            request_queue_capacity,
            MAX_PREPARATION_QUEUE_CAPACITY,
        )?;
        validate_capacity(
            "result_queue_capacity",
            result_queue_capacity,
            MAX_PREPARATION_QUEUE_CAPACITY,
        )?;
        validate_capacity("max_in_flight", max_in_flight, MAX_PREPARATION_CONCURRENCY)?;
        Ok(Self {
            request_queue_capacity,
            result_queue_capacity,
            max_in_flight,
        })
    }

    #[must_use]
    pub const fn request_queue_capacity(self) -> usize {
        self.request_queue_capacity
    }

    #[must_use]
    pub const fn result_queue_capacity(self) -> usize {
        self.result_queue_capacity
    }

    #[must_use]
    pub const fn max_in_flight(self) -> usize {
        self.max_in_flight
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PreparationWorkerSettingsError {
    #[error("preparation worker {field} must be greater than zero")]
    ZeroCapacity { field: &'static str },
    #[error("preparation worker {field} {value} exceeds maximum {maximum}")]
    CapacityTooLarge {
        field: &'static str,
        value: usize,
        maximum: usize,
    },
}

fn validate_capacity(
    field: &'static str,
    value: usize,
    maximum: usize,
) -> Result<(), PreparationWorkerSettingsError> {
    if value == 0 {
        return Err(PreparationWorkerSettingsError::ZeroCapacity { field });
    }
    if value > maximum {
        return Err(PreparationWorkerSettingsError::CapacityTooLarge {
            field,
            value,
            maximum,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct PreparationExecutionError {
    message: String,
}

impl PreparationExecutionError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub trait QuadrantPreparationExecutor: Send + Sync + 'static {
    fn execute(
        &self,
        identity: QuadrantPreparationIdentity,
    ) -> Result<PreparedQuadrantArtifact, PreparationExecutionError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreparationWorkerEvent {
    Completed(PreparedQuadrantArtifact),
    Failed {
        identity: QuadrantPreparationIdentity,
        error: PreparationExecutionError,
    },
}

impl PreparationWorkerEvent {
    #[must_use]
    pub const fn identity(&self) -> QuadrantPreparationIdentity {
        match self {
            Self::Completed(artifact) => artifact.identity(),
            Self::Failed { identity, .. } => *identity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparationSubmissionOutcome {
    Submitted,
    Duplicate,
    Full,
    Closed,
}

#[derive(Debug, Clone)]
pub struct PreparationWorkerIngress {
    requests: mpsc::Sender<QuadrantPreparationIdentity>,
    outstanding: Arc<Mutex<HashSet<QuadrantPreparationIdentity>>>,
}

impl PreparationWorkerIngress {
    #[must_use]
    pub fn try_submit(&self, identity: QuadrantPreparationIdentity) -> PreparationSubmissionOutcome {
        {
            let mut outstanding = lock_outstanding(&self.outstanding);
            if !outstanding.insert(identity) {
                return PreparationSubmissionOutcome::Duplicate;
            }
        }

        match self.requests.try_send(identity) {
            Ok(()) => PreparationSubmissionOutcome::Submitted,
            Err(mpsc::error::TrySendError::Full(_)) => {
                remove_outstanding(&self.outstanding, identity);
                PreparationSubmissionOutcome::Full
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                remove_outstanding(&self.outstanding, identity);
                PreparationSubmissionOutcome::Closed
            }
        }
    }

    #[must_use]
    pub fn remaining_queue_capacity(&self) -> usize {
        self.requests.capacity()
    }
}

#[derive(Debug)]
pub struct PreparationWorkerResults {
    results: mpsc::Receiver<PreparationWorkerEvent>,
}

impl PreparationWorkerResults {
    pub async fn recv(&mut self) -> Option<PreparationWorkerEvent> {
        self.results.recv().await
    }

    pub fn try_recv(&mut self) -> Result<PreparationWorkerEvent, mpsc::error::TryRecvError> {
        self.results.try_recv()
    }
}

#[derive(Debug)]
pub struct PreparationWorkerTask {
    join: JoinHandle<()>,
}

impl PreparationWorkerTask {
    pub async fn join(self) -> Result<(), JoinError> {
        self.join.await
    }
}

pub fn spawn_preparation_worker<E>(
    settings: PreparationWorkerSettings,
    executor: Arc<E>,
) -> (
    PreparationWorkerIngress,
    PreparationWorkerResults,
    PreparationWorkerTask,
)
where
    E: QuadrantPreparationExecutor,
{
    let (request_tx, request_rx) = mpsc::channel(settings.request_queue_capacity());
    let (result_tx, result_rx) = mpsc::channel(settings.result_queue_capacity());
    let outstanding = Arc::new(Mutex::new(HashSet::new()));
    let worker_outstanding = Arc::clone(&outstanding);
    let join = tokio::spawn(run_worker(
        request_rx,
        result_tx,
        settings.max_in_flight(),
        executor,
        worker_outstanding,
    ));

    (
        PreparationWorkerIngress {
            requests: request_tx,
            outstanding,
        },
        PreparationWorkerResults { results: result_rx },
        PreparationWorkerTask { join },
    )
}

async fn run_worker<E>(
    mut requests: mpsc::Receiver<QuadrantPreparationIdentity>,
    results: mpsc::Sender<PreparationWorkerEvent>,
    max_in_flight: usize,
    executor: Arc<E>,
    outstanding: Arc<Mutex<HashSet<QuadrantPreparationIdentity>>>,
) where
    E: QuadrantPreparationExecutor,
{
    let mut tasks = JoinSet::new();
    let mut input_closed = false;

    loop {
        if input_closed && tasks.is_empty() {
            break;
        }

        tokio::select! {
            request = requests.recv(), if !input_closed && tasks.len() < max_in_flight => {
                match request {
                    Some(identity) => {
                        let executor = Arc::clone(&executor);
                        tasks.spawn(async move {
                            let execution = tokio::task::spawn_blocking(move || executor.execute(identity)).await;
                            let result = match execution {
                                Ok(result) => result,
                                Err(error) => Err(PreparationExecutionError::new(format!(
                                    "quadrant preparation blocking task failed: {error}"
                                ))),
                            };
                            (identity, result)
                        });
                    }
                    None => input_closed = true,
                }
            }
            joined = tasks.join_next(), if !tasks.is_empty() => {
                let Some(joined) = joined else {
                    continue;
                };
                let Ok((identity, result)) = joined else {
                    clear_outstanding(&outstanding);
                    return;
                };
                let event = match result {
                    Ok(artifact) => PreparationWorkerEvent::Completed(artifact),
                    Err(error) => PreparationWorkerEvent::Failed { identity, error },
                };
                let delivered = results.send(event).await.is_ok();
                remove_outstanding(&outstanding, identity);
                if !delivered {
                    clear_outstanding(&outstanding);
                    return;
                }
            }
        }
    }
}

fn lock_outstanding(
    outstanding: &Mutex<HashSet<QuadrantPreparationIdentity>>,
) -> MutexGuard<'_, HashSet<QuadrantPreparationIdentity>> {
    match outstanding.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn remove_outstanding(
    outstanding: &Mutex<HashSet<QuadrantPreparationIdentity>>,
    identity: QuadrantPreparationIdentity,
) {
    lock_outstanding(outstanding).remove(&identity);
}

fn clear_outstanding(outstanding: &Mutex<HashSet<QuadrantPreparationIdentity>>) {
    lock_outstanding(outstanding).clear();
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        io,
        sync::{
            Condvar,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use aurenfall_core::QuadrantCoord;
    use aurenfall_domain::{
        FrontierQuadrantState, FrontierState, InitialFrontier, PreparationFingerprint,
        QuadrantPreparationAbandonOutcome, QuadrantPreparationCompletionOutcome,
        QuadrantPreparationRequestOutcome, QuadrantPreparationState, derive_candidate_frontier,
    };

    use super::*;

    #[derive(Debug)]
    struct FastExecutor;

    impl QuadrantPreparationExecutor for FastExecutor {
        fn execute(
            &self,
            identity: QuadrantPreparationIdentity,
        ) -> Result<PreparedQuadrantArtifact, PreparationExecutionError> {
            let fingerprint = PreparationFingerprint::new([0xA5; 32])
                .map_err(|error| PreparationExecutionError::new(error.to_string()))?;
            PreparedQuadrantArtifact::new(identity, fingerprint, 64)
                .map_err(|error| PreparationExecutionError::new(error.to_string()))
        }
    }

    #[derive(Debug)]
    struct FailingExecutor;

    impl QuadrantPreparationExecutor for FailingExecutor {
        fn execute(
            &self,
            _identity: QuadrantPreparationIdentity,
        ) -> Result<PreparedQuadrantArtifact, PreparationExecutionError> {
            Err(PreparationExecutionError::new("synthetic preparation failure"))
        }
    }

    #[derive(Debug)]
    struct BlockingExecutor {
        started: Arc<AtomicUsize>,
        gate: Arc<(Mutex<bool>, Condvar)>,
    }

    impl QuadrantPreparationExecutor for BlockingExecutor {
        fn execute(
            &self,
            identity: QuadrantPreparationIdentity,
        ) -> Result<PreparedQuadrantArtifact, PreparationExecutionError> {
            self.started.fetch_add(1, Ordering::SeqCst);
            let (gate, condition) = &*self.gate;
            let mut released = match gate.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            while !*released {
                released = match condition.wait(released) {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
            }
            FastExecutor.execute(identity)
        }
    }

    fn identity(x: i64, input: u8) -> Result<QuadrantPreparationIdentity, Box<dyn Error>> {
        let fingerprint = PreparationFingerprint::new([input; 32])?;
        Ok(QuadrantPreparationIdentity::new(
            QuadrantCoord::new(x, 0),
            7,
            fingerprint,
        )?)
    }

    fn frontier_fixture() -> Result<(FrontierState, QuadrantPreparationState), Box<dyn Error>> {
        let initial = InitialFrontier::rectangular(QuadrantCoord::new(-1, -1), 4, 4)?;
        let mut frontier = FrontierState::from_initial(&initial, 1)?;
        derive_candidate_frontier(&mut frontier)?;
        Ok((frontier, QuadrantPreparationState::default()))
    }

    async fn wait_for_started(started: &AtomicUsize, expected: usize) -> Result<(), Box<dyn Error>> {
        tokio::time::timeout(Duration::from_secs(2), async {
            while started.load(Ordering::SeqCst) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await?;
        Ok(())
    }

    fn release(gate: &Arc<(Mutex<bool>, Condvar)>) {
        let (lock, condition) = &**gate;
        let mut released = match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *released = true;
        condition.notify_all();
    }

    #[test]
    fn settings_reject_zero_and_unbounded_values() {
        assert!(matches!(
            PreparationWorkerSettings::new(0, 1, 1),
            Err(PreparationWorkerSettingsError::ZeroCapacity {
                field: "request_queue_capacity"
            })
        ));
        assert!(matches!(
            PreparationWorkerSettings::new(1, 1, MAX_PREPARATION_CONCURRENCY + 1),
            Err(PreparationWorkerSettingsError::CapacityTooLarge {
                field: "max_in_flight",
                ..
            })
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bounded_ingress_reports_backpressure_without_silent_drop() -> Result<(), Box<dyn Error>> {
        let settings = PreparationWorkerSettings::new(1, 4, 1)?;
        let started = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let executor = Arc::new(BlockingExecutor {
            started: Arc::clone(&started),
            gate: Arc::clone(&gate),
        });
        let (ingress, mut results, task) = spawn_preparation_worker(settings, executor);
        let first = identity(3, 1)?;
        let second = identity(4, 2)?;
        let third = identity(5, 3)?;

        assert_eq!(ingress.try_submit(first), PreparationSubmissionOutcome::Submitted);
        wait_for_started(&started, 1).await?;
        assert_eq!(
            ingress.try_submit(second),
            PreparationSubmissionOutcome::Submitted
        );
        assert_eq!(ingress.try_submit(third), PreparationSubmissionOutcome::Full);

        release(&gate);
        drop(ingress);
        let first_event = results
            .recv()
            .await
            .ok_or_else(|| io::Error::other("missing first preparation result"))?;
        let second_event = results
            .recv()
            .await
            .ok_or_else(|| io::Error::other("missing second preparation result"))?;
        task.join().await?;

        let identities = [first_event.identity(), second_event.identity()];
        assert!(identities.contains(&first));
        assert!(identities.contains(&second));
        assert!(!identities.contains(&third));
        assert!(results.recv().await.is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn duplicate_identity_is_suppressed_before_duplicate_generation() -> Result<(), Box<dyn Error>> {
        let settings = PreparationWorkerSettings::new(2, 2, 1)?;
        let started = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let executor = Arc::new(BlockingExecutor {
            started: Arc::clone(&started),
            gate: Arc::clone(&gate),
        });
        let (ingress, mut results, task) = spawn_preparation_worker(settings, executor);
        let request = identity(3, 1)?;

        assert_eq!(
            ingress.try_submit(request),
            PreparationSubmissionOutcome::Submitted
        );
        wait_for_started(&started, 1).await?;
        assert_eq!(
            ingress.try_submit(request),
            PreparationSubmissionOutcome::Duplicate
        );
        release(&gate);
        drop(ingress);

        let event = results
            .recv()
            .await
            .ok_or_else(|| io::Error::other("missing preparation result"))?;
        assert_eq!(event.identity(), request);
        task.join().await?;
        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert!(results.recv().await.is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn configured_concurrency_caps_started_blocking_jobs() -> Result<(), Box<dyn Error>> {
        let settings = PreparationWorkerSettings::new(4, 4, 2)?;
        let started = Arc::new(AtomicUsize::new(0));
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let executor = Arc::new(BlockingExecutor {
            started: Arc::clone(&started),
            gate: Arc::clone(&gate),
        });
        let (ingress, mut results, task) = spawn_preparation_worker(settings, executor);

        for (x, input) in [(3, 1), (4, 2), (5, 3)] {
            assert_eq!(
                ingress.try_submit(identity(x, input)?),
                PreparationSubmissionOutcome::Submitted
            );
        }
        wait_for_started(&started, 2).await?;
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert_eq!(started.load(Ordering::SeqCst), 2);

        release(&gate);
        drop(ingress);
        for _ in 0..3 {
            let _ = results
                .recv()
                .await
                .ok_or_else(|| io::Error::other("missing bounded concurrency result"))?;
        }
        task.join().await?;
        assert_eq!(started.load(Ordering::SeqCst), 3);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worker_result_requires_authoritative_owner_commit_to_become_prepared()
    -> Result<(), Box<dyn Error>> {
        let (mut frontier, mut preparations) = frontier_fixture()?;
        let request = identity(3, 1)?;
        assert_eq!(
            preparations.request(&frontier, request)?,
            QuadrantPreparationRequestOutcome::Started
        );
        let settings = PreparationWorkerSettings::new(2, 2, 1)?;
        let (ingress, mut results, task) = spawn_preparation_worker(settings, Arc::new(FastExecutor));

        assert_eq!(
            ingress.try_submit(request),
            PreparationSubmissionOutcome::Submitted
        );
        let event = results
            .recv()
            .await
            .ok_or_else(|| io::Error::other("missing preparation result"))?;
        let PreparationWorkerEvent::Completed(artifact) = event else {
            return Err(io::Error::other("expected completed preparation artifact").into());
        };

        assert_eq!(frontier.state(request.coord()), FrontierQuadrantState::Candidate);
        assert_eq!(frontier.revision(), 1);
        assert!(matches!(
            preparations.complete(&mut frontier, artifact)?,
            QuadrantPreparationCompletionOutcome::Prepared { .. }
        ));
        assert_eq!(frontier.state(request.coord()), FrontierQuadrantState::Prepared);
        assert_eq!(frontier.revision(), 1);

        drop(ingress);
        task.join().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worker_failure_leaves_candidate_until_authoritative_owner_handles_failure()
    -> Result<(), Box<dyn Error>> {
        let (frontier, mut preparations) = frontier_fixture()?;
        let request = identity(3, 1)?;
        preparations.request(&frontier, request)?;
        let settings = PreparationWorkerSettings::new(2, 2, 1)?;
        let (ingress, mut results, task) = spawn_preparation_worker(settings, Arc::new(FailingExecutor));

        assert_eq!(
            ingress.try_submit(request),
            PreparationSubmissionOutcome::Submitted
        );
        let event = results
            .recv()
            .await
            .ok_or_else(|| io::Error::other("missing preparation failure"))?;
        assert!(matches!(
            event,
            PreparationWorkerEvent::Failed { identity, .. } if identity == request
        ));
        assert_eq!(frontier.state(request.coord()), FrontierQuadrantState::Candidate);
        assert_eq!(frontier.revision(), 1);
        assert_eq!(
            preparations.fail(request)?,
            QuadrantPreparationAbandonOutcome::Removed
        );
        assert_eq!(frontier.state(request.coord()), FrontierQuadrantState::Candidate);
        assert_eq!(frontier.revision(), 1);

        drop(ingress);
        task.join().await?;
        Ok(())
    }
}
