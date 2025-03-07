use super::Task;
use crate::cloud_provider::aws::regions::AwsRegion;
use crate::cmd::docker::Docker;
use crate::engine::EngineConfigError;
use crate::errors::EngineError;
use crate::events::Stage::Infrastructure;
use crate::events::{EngineEvent, EventDetails, EventMessage, InfrastructureStep, Transmitter};
use crate::io_models::context::Context;
use crate::io_models::engine_request::InfrastructureEngineRequest;
use crate::io_models::{Action, QoveryIdentifier};
use crate::logger::Logger;
use crate::transaction::{Transaction, TransactionResult};
use chrono::{DateTime, Utc};
use std::{env, fs};
use url::Url;

#[derive(Clone)]
pub struct InfrastructureTask {
    workspace_root_dir: String,
    lib_root_dir: String,
    docker_host: Option<Url>,
    docker: Docker,
    request: InfrastructureEngineRequest,
    logger: Box<dyn Logger>,
}

impl InfrastructureTask {
    pub fn new(
        request: InfrastructureEngineRequest,
        workspace_root_dir: String,
        lib_root_dir: String,
        docker_host: Option<Url>,
        logger: Box<dyn Logger>,
    ) -> Self {
        let docker = Docker::new(docker_host.clone()).expect("Can't init docker builder");
        InfrastructureTask {
            workspace_root_dir,
            lib_root_dir,
            docker_host,
            docker,
            request,
            logger,
        }
    }

    fn info_context(&self) -> Context {
        Context::new(
            self.request.organization_long_id,
            self.request.kubernetes.long_id,
            self.request.id.to_string(),
            self.workspace_root_dir.to_string(),
            self.lib_root_dir.to_string(),
            self.request.test_cluster,
            self.docker_host.clone(),
            self.request.features.clone(),
            self.request.metadata.clone(),
            self.docker.clone(),
            self.request.event_details(),
        )
    }

    fn handle_transaction_result(&self, logger: Box<dyn Logger>, transaction_result: TransactionResult) {
        match transaction_result {
            TransactionResult::Ok => {
                self.send_infrastructure_progress(logger.clone(), None);
            }
            TransactionResult::Error(engine_error) => {
                self.send_infrastructure_progress(logger.clone(), Some(*engine_error));
            }
            TransactionResult::Canceled => {
                // should never happen by design
                error!("Infrastructure task should never been canceled");
            }
        }
    }

    fn send_infrastructure_progress(&self, logger: Box<dyn Logger>, option_engine_error: Option<EngineError>) {
        let kubernetes = &self.request.kubernetes;
        if let Some(engine_error) = option_engine_error {
            let infrastructure_step = match self.request.action {
                Action::Create => InfrastructureStep::CreateError,
                Action::Pause => InfrastructureStep::PauseError,
                Action::Delete => InfrastructureStep::DeleteError,
            };
            let event_message =
                EventMessage::new_from_safe(format!("Kubernetes cluster failure {}", &infrastructure_step));

            let engine_event = EngineEvent::Error(
                engine_error.clone_engine_error_with_stage(Infrastructure(infrastructure_step)),
                Some(event_message),
            );

            logger.log(engine_event);
        } else {
            let infrastructure_step = match self.request.action {
                Action::Create => InfrastructureStep::Created,
                Action::Pause => InfrastructureStep::Paused,
                Action::Delete => InfrastructureStep::Deleted,
            };
            let event_message =
                EventMessage::new_from_safe(format!("Kubernetes cluster successfully {}", &infrastructure_step));
            let engine_event = EngineEvent::Info(
                EventDetails::new(
                    Some(self.request.cloud_provider.kind.clone()),
                    QoveryIdentifier::new(self.request.organization_long_id),
                    QoveryIdentifier::new(kubernetes.long_id),
                    self.request.id.to_string(),
                    Infrastructure(infrastructure_step),
                    Transmitter::Kubernetes(kubernetes.long_id, kubernetes.name.to_string()),
                ),
                event_message,
            );

            logger.log(engine_event);
        }
    }
}

impl Task for InfrastructureTask {
    fn created_at(&self) -> &DateTime<Utc> {
        &self.request.created_at
    }

    fn id(&self) -> &str {
        self.request.id.as_str()
    }

    fn run(&self) {
        info!(
            "infrastructure task {} started with infrastructure id {}-{}-{}",
            self.id(),
            self.request.cloud_provider.id.as_str(),
            self.request.container_registry.id.as_str(),
            self.request.build_platform.id.as_str()
        );

        let engine = match self
            .request
            .engine(&self.info_context(), self.request.event_details(), self.logger.clone())
        {
            Ok(engine) => engine,
            Err(err) => {
                self.send_infrastructure_progress(self.logger.clone(), Some(err));
                return;
            }
        };

        // check and init the connection to all services
        let mut tx = match Transaction::new(&engine) {
            Ok(transaction) => transaction,
            Err(err) => {
                let engine_error = match err {
                    EngineConfigError::BuildPlatformNotValid(engine_error) => engine_error,
                    EngineConfigError::CloudProviderNotValid(engine_error) => engine_error,
                    EngineConfigError::DnsProviderNotValid(engine_error) => engine_error,
                    EngineConfigError::KubernetesNotValid(engine_error) => engine_error,
                };
                self.send_infrastructure_progress(self.logger.clone(), Some(engine_error));
                return;
            }
        };

        let _ = match self.request.action {
            Action::Create => tx.create_kubernetes(),
            Action::Pause => tx.pause_kubernetes(),
            Action::Delete => tx.delete_kubernetes(),
        };

        self.handle_transaction_result(self.logger.clone(), tx.commit());

        // only store if not running on a workstation
        if env::var("DEPLOY_FROM_FILE_KIND").is_err() {
            match crate::fs::create_workspace_archive(
                engine.context().workspace_root_dir(),
                engine.context().execution_id(),
            ) {
                Ok(file) => match super::upload_s3_file(
                    &self.info_context(),
                    self.request.archive.as_ref(),
                    file.as_str(),
                    AwsRegion::EuWest3, // TODO(benjaminch): make it customizable
                    self.request.kubernetes.advanced_settings.pleco_resources_ttl,
                ) {
                    Ok(_) => {
                        let _ = fs::remove_file(file).map_err(|err| error!("Cannot delete file {}", err));
                    }
                    Err(e) => error!("Error while uploading archive {}", e),
                },
                Err(err) => error!("{}", err),
            };
        };

        info!("infrastructure task {} finished", self.id());
    }

    fn cancel(&self) -> bool {
        false
    }

    fn cancel_checker(&self) -> Box<dyn Fn() -> bool> {
        Box::new(|| false)
    }
}
