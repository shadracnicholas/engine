use crate::deployment_report::logger::{EnvLogger, EnvProgressLogger, EnvSuccessLogger};
use crate::errors::EngineError;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::Duration;

pub mod application;
pub mod database;
pub mod job;
pub mod logger;
pub mod router;
mod utils;

// Object responsible to log the progress of a deployment
// This object is going to live in his own thread and is responsible to
// 1. Fetch information of the deployment
// 2. Render those information in a human readable way
// 3. Send those information to the logger
// The mut Self::Deployment is a state where we externalize the mutable state of the reporter.
// As the object is shared among 2 threads, we can't have 2 mutable references to the same object.
// So the reporter state that is going to be mutated is externalized and repass at every call to avoid that
pub trait DeploymentReporter: Send + Sync {
    type DeploymentResult;
    type DeploymentState: Send;
    type Logger;

    fn logger(&self) -> &Self::Logger;
    fn new_state(&self) -> Self::DeploymentState;
    fn deployment_before_start(&self, state: &mut Self::DeploymentState);
    fn deployment_in_progress(&self, state: &mut Self::DeploymentState);
    fn deployment_terminated(
        &self,
        result: &Result<Self::DeploymentResult, EngineError>,
        state: &mut Self::DeploymentState,
    );
    fn report_frequency(&self) -> Duration {
        Duration::from_secs(10)
    }
}

// This object represent a complex deployment task that is supposed to be long running and used with a reporter.
// We split pre_run and run, in order to allow the reporter to not be executed/log while the pre_run is running.
// Reporter will log/only be executed when the task is executing the run method.
pub trait DeploymentTask {
    type Logger;
    type DeploymentResult;

    fn pre_run(&self, logger: &Self::Logger) -> Result<Self::DeploymentResult, EngineError>;
    fn run(&self, logger: &Self::Logger, state: Self::DeploymentResult) -> Result<Self::DeploymentResult, EngineError>;
    fn post_run_success(&self, logger: &Self::Logger, state: Self::DeploymentResult);
}

pub struct DeploymentTaskImpl<'a, Pre, Run, Post, Ret>
where
    Pre: Fn(&EnvProgressLogger) -> Result<Ret, EngineError>,
    Run: Fn(&EnvProgressLogger, Ret) -> Result<Ret, EngineError>,
    Post: Fn(&EnvSuccessLogger, Ret),
{
    pub pre_run: &'a Pre,
    pub run: &'a Run,
    pub post_run_success: &'a Post,
}

impl<'a, Pre, Run, Post, Ret> DeploymentTask for DeploymentTaskImpl<'a, Pre, Run, Post, Ret>
where
    Pre: Fn(&EnvProgressLogger) -> Result<Ret, EngineError>,
    Run: Fn(&EnvProgressLogger, Ret) -> Result<Ret, EngineError>,
    Post: Fn(&EnvSuccessLogger, Ret),
{
    type Logger = EnvLogger;
    type DeploymentResult = Ret;

    fn pre_run(&self, logger: &Self::Logger) -> Result<Self::DeploymentResult, EngineError> {
        let progress_logger = EnvProgressLogger::new(logger);
        (self.pre_run)(&progress_logger)
    }

    fn run(&self, logger: &Self::Logger, state: Self::DeploymentResult) -> Result<Self::DeploymentResult, EngineError> {
        let progress_logger = EnvProgressLogger::new(logger);
        (self.run)(&progress_logger, state)
    }

    fn post_run_success(&self, logger: &Self::Logger, state: Self::DeploymentResult) {
        let success_logger = EnvSuccessLogger::new(logger);
        (self.post_run_success)(&success_logger, state)
    }
}

// Blanket impl helper to create a deployment task from a closure
impl<T> DeploymentTask for T
where
    T: Fn(&EnvProgressLogger) -> Result<(), EngineError>,
{
    type Logger = EnvLogger;
    type DeploymentResult = ();

    fn pre_run(&self, _logger: &Self::Logger) -> Result<Self::DeploymentResult, EngineError> {
        Ok(())
    }

    fn run(&self, logger: &Self::Logger, state: Self::DeploymentResult) -> Result<Self::DeploymentResult, EngineError> {
        let progress_logger = EnvProgressLogger::new(logger);
        match self(&progress_logger) {
            Ok(_) => Ok(state),
            Err(e) => Err(e),
        }
    }

    fn post_run_success(&self, _logger: &Self::Logger, _state: Self::DeploymentResult) {}
}

// Function that take a deployment reporter and a deployment task and execute/synchronize them together
// The reporter is going to be executed in a separate thread and the task in the current thread.
// Reporter will not be executed while the task is running the pre_run and post_run_success methods.
// Only during the run method
pub fn execute_long_deployment<Log, TaskRet>(
    deployment_reporter: impl DeploymentReporter<DeploymentResult = TaskRet, Logger = Log>,
    long_task: impl DeploymentTask<Logger = Log, DeploymentResult = TaskRet>,
) -> Result<(), EngineError> {
    // stop the thread when the blocking task is done
    let (tx, rx) = mpsc::channel();
    let deployment_start = Arc::new(Barrier::new(2));
    let mut state = deployment_reporter.new_state();

    let logger = deployment_reporter.logger();
    let action_state = long_task.pre_run(logger)?;

    let deployment_result = thread::scope(|th_scope| {
        // monitor thread to notify user while the blocking task is executed
        let th_handle = thread::Builder::new()
            .name("deployment-monitor".to_string())
            .spawn_scoped(th_scope, {
                // Propagate the current span into the thread. This span is only used by tests
                let current_span = tracing::Span::current();
                let deployment_start = deployment_start.clone();
                let deployment_reporter = &deployment_reporter; // to avoid moving the object into the thread
                let state = &mut state;

                move || {
                    let _span = current_span.enter();

                    // Before the launch of the deployment
                    deployment_reporter.deployment_before_start(state);

                    // Wait the start of the deployment
                    deployment_start.wait();

                    // Send deployment progress report every x secs
                    let report_frequency = deployment_reporter.report_frequency();
                    loop {
                        match rx.recv_timeout(report_frequency) {
                            // Deployment is terminated, we received the result of the task
                            Ok(_) => break,

                            // Deployment is still in progress
                            Err(RecvTimeoutError::Timeout) => deployment_reporter.deployment_in_progress(state),

                            // Other side died without passing us the result ! this is a logical bug !
                            Err(RecvTimeoutError::Disconnected) => {
                                panic!(
                                    "Haven't received task deployment result, but otherside of the channel is dead !"
                                );
                            }
                        }
                    }
                }
            });

        // Wait for our watcher thread to be ready before starting
        let _ = deployment_start.wait();
        let deployment_result = long_task.run(deployment_reporter.logger(), action_state);
        let _ = tx.send(());
        let _ = th_handle.map(|th| th.join()); // wait for the thread to terminate

        deployment_result
    });

    deployment_reporter.deployment_terminated(&deployment_result, &mut state);
    match deployment_result {
        Ok(ret) => {
            long_task.post_run_success(deployment_reporter.logger(), ret);
            Ok(())
        }
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod test {
    use crate::deployment_report::{execute_long_deployment, DeploymentReporter, DeploymentTask};
    use crate::errors::EngineError;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    pub struct DeploymentReporterTest {
        pub before_deployment: Arc<AtomicBool>,
        pub deployment_in_progress: Arc<AtomicBool>,
        pub deployment_terminated: Arc<AtomicBool>,
        pub thread_dead: Arc<AtomicBool>,
        pub is_task_started: Arc<AtomicBool>,
    }

    impl Drop for DeploymentReporterTest {
        fn drop(&mut self) {
            self.thread_dead.store(true, Ordering::SeqCst)
        }
    }

    impl DeploymentReporter for DeploymentReporterTest {
        type DeploymentResult = u32;
        type DeploymentState = ();
        type Logger = ();

        fn logger(&self) -> &Self::Logger {
            &()
        }

        fn new_state(&self) -> Self::DeploymentState {}

        fn deployment_before_start(&self, _: &mut Self::DeploymentState) {
            assert!(!self.is_task_started.load(Ordering::SeqCst));
            self.before_deployment.store(true, Ordering::SeqCst)
        }

        fn deployment_in_progress(&self, _: &mut Self::DeploymentState) {
            self.deployment_in_progress.store(true, Ordering::SeqCst)
        }

        fn deployment_terminated(
            &self,
            _result: &Result<Self::DeploymentResult, EngineError>,
            _: &mut Self::DeploymentState,
        ) {
            self.deployment_terminated.store(true, Ordering::SeqCst);
        }

        fn report_frequency(&self) -> Duration {
            Duration::from_secs(1)
        }
    }

    struct DeploymentAction {
        pub run_fn: Box<dyn Fn()>,
        pub pre_run: Arc<AtomicBool>,
        pub run: Arc<AtomicBool>,
        pub post_run: Arc<AtomicBool>,
    }

    impl DeploymentTask for DeploymentAction {
        type Logger = ();
        type DeploymentResult = u32;

        fn pre_run(&self, _logger: &Self::Logger) -> Result<Self::DeploymentResult, EngineError> {
            self.pre_run.store(true, Ordering::SeqCst);
            Ok(1)
        }

        fn run(
            &self,
            _logger: &Self::Logger,
            state: Self::DeploymentResult,
        ) -> Result<Self::DeploymentResult, EngineError> {
            self.run.store(true, Ordering::SeqCst);
            (self.run_fn)();
            assert_eq!(state, 1);
            Ok(2)
        }

        fn post_run_success(&self, _logger: &Self::Logger, state: Self::DeploymentResult) {
            self.post_run.store(true, Ordering::SeqCst);
            assert_eq!(state, 2);
        }
    }

    #[test]
    fn test_execute_long_deployment() {
        let reporter = DeploymentReporterTest {
            before_deployment: Arc::new(AtomicBool::new(false)),
            deployment_in_progress: Arc::new(AtomicBool::new(false)),
            deployment_terminated: Arc::new(AtomicBool::new(false)),
            thread_dead: Arc::new(AtomicBool::new(false)),
            is_task_started: Arc::new(AtomicBool::new(false)),
        };

        let before_deployment = reporter.before_deployment.clone();
        let deployment_in_progress = reporter.deployment_in_progress.clone();
        let deployment_terminated = reporter.deployment_terminated.clone();
        let thread_dead = reporter.thread_dead.clone();
        let is_task_started = reporter.is_task_started.clone();

        let task = DeploymentAction {
            run_fn: Box::new(move || {
                is_task_started.store(true, Ordering::SeqCst);
                thread::sleep(Duration::from_secs(2));
            }),
            pre_run: Default::default(),
            run: Default::default(),
            post_run: Default::default(),
        };

        let pre_run = task.pre_run.clone();
        let run = task.run.clone();
        let post_run = task.post_run.clone();
        let _ = execute_long_deployment(reporter, task);

        // Check that our method have been called
        assert!(before_deployment.load(Ordering::SeqCst));
        assert!(deployment_in_progress.load(Ordering::SeqCst));
        assert!(thread_dead.load(Ordering::SeqCst));
        assert!(deployment_terminated.load(Ordering::SeqCst));

        // Check that our method have been called
        assert!(pre_run.load(Ordering::SeqCst));
        assert!(run.load(Ordering::SeqCst));
        assert!(post_run.load(Ordering::SeqCst));
    }
}
