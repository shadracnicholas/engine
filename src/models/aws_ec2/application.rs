use crate::cloud_provider::kubernetes::validate_k8s_required_cpu_and_burstable;
use crate::cloud_provider::models::StorageDataTemplate;
use crate::cloud_provider::DeploymentTarget;
use crate::errors::EngineError;
use crate::events::{EnvironmentStep, Stage};
use crate::models::application::Application;
use crate::models::aws_ec2::AwsEc2StorageType;
use crate::models::types::{AWSEc2, ToTeraContext};
use tera::Context as TeraContext;

impl ToTeraContext for Application<AWSEc2> {
    fn to_tera_context(&self, target: &DeploymentTarget) -> Result<TeraContext, EngineError> {
        let event_details = (self.mk_event_details)(Stage::Environment(EnvironmentStep::LoadConfiguration));
        let mut context = self.default_tera_context(target.kubernetes, target.environment);

        let cpu_limits = match validate_k8s_required_cpu_and_burstable(self.total_cpus(), self.cpu_burst()) {
            Ok(l) => l,
            Err(e) => {
                return Err(EngineError::new_k8s_validate_required_cpu_and_burstable_error(
                    event_details,
                    self.total_cpus(),
                    self.cpu_burst(),
                    e,
                ));
            }
        };
        context.insert("cpu_burst", &cpu_limits.cpu_limit);

        let storage = self
            .storage
            .iter()
            .map(|s| StorageDataTemplate {
                id: s.id.clone(),
                long_id: self.long_id,
                name: s.name.clone(),
                storage_type: match s.storage_type {
                    AwsEc2StorageType::SC1 => "sc1",
                    AwsEc2StorageType::ST1 => "st1",
                    AwsEc2StorageType::GP2 => "gp2",
                    AwsEc2StorageType::IO1 => "io1",
                }
                .to_string(),
                size_in_gib: s.size_in_gib,
                mount_point: s.mount_point.clone(),
                snapshot_retention_in_days: s.snapshot_retention_in_days,
            })
            .collect::<Vec<_>>();

        let is_storage = !storage.is_empty();

        context.insert("storage", &storage);
        context.insert("is_storage", &is_storage);

        Ok(context)
    }
}
