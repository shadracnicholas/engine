use crate::engine::{EngineConfigError, InfrastructureContext};
use crate::errors::EngineError;

pub struct Transaction<'a> {
    engine: &'a InfrastructureContext,
    steps: Vec<Step>,
    executed_steps: Vec<Step>,
}

impl<'a> Transaction<'a> {
    pub fn new(engine: &'a InfrastructureContext) -> Result<Self, EngineConfigError> {
        engine.is_valid()?;
        if let Err(e) = engine.kubernetes().is_valid() {
            return Err(EngineConfigError::KubernetesNotValid(e));
        }

        Ok(Transaction::<'a> {
            engine,
            steps: vec![],
            executed_steps: vec![],
        })
    }

    pub fn create_kubernetes(&mut self) -> Result<(), EngineError> {
        self.steps.push(Step::CreateKubernetes);
        Ok(())
    }

    pub fn pause_kubernetes(&mut self) -> Result<(), EngineError> {
        self.steps.push(Step::PauseKubernetes);
        Ok(())
    }

    pub fn delete_kubernetes(&mut self) -> Result<(), EngineError> {
        self.steps.push(Step::DeleteKubernetes);
        Ok(())
    }

    pub fn rollback(&self) -> Result<(), RollbackError> {
        for step in self.executed_steps.iter() {
            match step {
                Step::CreateKubernetes => {
                    // revert kubernetes creation
                    if let Err(err) = self.engine.kubernetes().on_create_error() {
                        return Err(RollbackError::CommitError(Box::new(err)));
                    };
                }
                Step::DeleteKubernetes => {
                    // revert kubernetes deletion
                    if let Err(err) = self.engine.kubernetes().on_delete_error() {
                        return Err(RollbackError::CommitError(Box::new(err)));
                    };
                }
                Step::PauseKubernetes => {
                    // revert pause
                    if let Err(err) = self.engine.kubernetes().on_pause_error() {
                        return Err(RollbackError::CommitError(Box::new(err)));
                    };
                }
            }
        }

        Ok(())
    }

    pub fn commit(mut self) -> TransactionResult {
        for step in self.steps.clone().into_iter() {
            // execution loop
            self.executed_steps.push(step.clone());

            match step {
                Step::CreateKubernetes => {
                    // create kubernetes
                    match self.commit_infrastructure(self.engine.kubernetes().on_create()) {
                        TransactionResult::Ok => {}
                        err => {
                            error!("Error while creating infrastructure: {:?}", err);
                            return err;
                        }
                    };
                }
                Step::DeleteKubernetes => {
                    // delete kubernetes
                    match self.commit_infrastructure(self.engine.kubernetes().on_delete()) {
                        TransactionResult::Ok => {}
                        err => {
                            error!("Error while deleting infrastructure: {:?}", err);
                            return err;
                        }
                    };
                }
                Step::PauseKubernetes => {
                    // pause kubernetes
                    match self.commit_infrastructure(self.engine.kubernetes().on_pause()) {
                        TransactionResult::Ok => {}
                        err => {
                            error!("Error while pausing infrastructure: {:?}", err);
                            return err;
                        }
                    };
                }
            };
        }

        TransactionResult::Ok
    }

    fn commit_infrastructure(&self, result: Result<(), EngineError>) -> TransactionResult {
        match result {
            Err(err) => {
                warn!("infrastructure ROLLBACK STARTED! an error occurred {:?}", err);
                match self.rollback() {
                    Ok(_) => {
                        // an error occurred on infrastructure deployment BUT rolledback is OK
                        TransactionResult::Error(Box::new(err))
                    }
                    Err(e) => {
                        // an error occurred on infrastructure deployment AND rolledback is KO
                        error!("infrastructure ROLLBACK FAILED! fatal error: {:?}", e);
                        TransactionResult::Error(Box::new(err))
                    }
                }
            }
            _ => {
                // infrastructure deployment OK
                TransactionResult::Ok
            }
        }
    }
}

#[derive(Clone)]
pub struct DeploymentOption {
    pub force_build: bool,
    pub force_push: bool,
}

#[derive(Clone)]
pub enum StepName {
    CreateKubernetes,
    DeleteKubernetes,
    PauseKubernetes,
    BuildEnvironment,
    DeployEnvironment,
    PauseEnvironment,
    DeleteEnvironment,
    Waiting,
}

impl StepName {
    pub fn can_be_canceled(&self) -> bool {
        match self {
            StepName::CreateKubernetes => false,
            StepName::DeleteKubernetes => false,
            StepName::PauseKubernetes => false,
            StepName::DeployEnvironment => true,
            StepName::PauseEnvironment => true,
            StepName::DeleteEnvironment => true,
            StepName::BuildEnvironment => true,
            StepName::Waiting => true,
        }
    }
}

pub enum Step {
    // init and create all the necessary resources (Network, Kubernetes)
    CreateKubernetes,
    DeleteKubernetes,
    PauseKubernetes,
}

impl Clone for Step {
    fn clone(&self) -> Self {
        match self {
            Step::CreateKubernetes => Step::CreateKubernetes,
            Step::DeleteKubernetes => Step::DeleteKubernetes,
            Step::PauseKubernetes => Step::PauseKubernetes,
        }
    }
}

#[derive(Debug)]
pub enum RollbackError {
    CommitError(Box<EngineError>),
    NoFailoverEnvironment,
    Nothing,
}

#[derive(Debug)]
pub enum TransactionResult {
    Ok,
    Canceled,
    Error(Box<EngineError>),
}
