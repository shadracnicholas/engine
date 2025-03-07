use crate::cloud_provider::helm::ChartInfo;
use crate::cloud_provider::DeploymentTarget;
use crate::cmd::command::CommandKiller;
use crate::deployment_action::DeploymentAction;
use crate::errors::EngineError;
use crate::events::EventDetails;
use crate::runtime::block_on;
use crate::template::generate_and_copy_all_files_into_dir;
use k8s_openapi::api::core::v1::Pod;
use kube::api::ListParams;
use kube::Api;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tera::Context as TeraContext;
use tokio::time::Instant;

const DEFAULT_HELM_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Helm Deployment manages Helm + jinja support
pub struct HelmDeployment {
    event_details: EventDetails,
    tera_context: TeraContext,
    /// The chart source directory which will be copied into the workspace
    chart_orginal_dir: PathBuf,
    /// name of the value files to render and use during helm deploy
    pub render_custom_values_file: Option<PathBuf>,
    /// Path should be inside the workspace directory because it will be copied there
    pub helm_chart: ChartInfo,
}

impl HelmDeployment {
    pub fn new(
        event_details: EventDetails,
        tera_context: TeraContext,
        chart_orginal_dir: PathBuf,
        render_custom_values_file: Option<PathBuf>,
        helm_chart: ChartInfo,
    ) -> HelmDeployment {
        HelmDeployment {
            event_details,
            tera_context,
            chart_orginal_dir,
            render_custom_values_file,
            helm_chart,
        }
    }

    pub fn prepare_helm_chart(&self) -> Result<(), EngineError> {
        // Copy the root folder
        generate_and_copy_all_files_into_dir(&self.chart_orginal_dir, &self.helm_chart.path, self.tera_context.clone())
            .map_err(|e| {
                EngineError::new_cannot_copy_files_from_one_directory_to_another(
                    self.event_details.clone(),
                    self.chart_orginal_dir.to_string_lossy().to_string(),
                    self.helm_chart.path.clone(),
                    e,
                )
            })?;

        // If we have some special value override, render and copy it
        if let Some(custom_value) = self.render_custom_values_file.clone() {
            let custom_value_dir_path = custom_value.parent().unwrap_or_else(|| Path::new("./"));

            generate_and_copy_all_files_into_dir(
                custom_value_dir_path,
                &self.helm_chart.path,
                self.tera_context.clone(),
            )
            .map_err(|e| {
                EngineError::new_cannot_copy_files_from_one_directory_to_another(
                    self.event_details.clone(),
                    self.chart_orginal_dir.to_string_lossy().to_string(),
                    self.helm_chart.path.clone(),
                    e,
                )
            })?;
        }

        Ok(())
    }
}

impl DeploymentAction for HelmDeployment {
    fn on_create(&self, target: &DeploymentTarget) -> Result<(), EngineError> {
        self.prepare_helm_chart()?;

        // print diff in logs
        let _ = target.helm.upgrade_diff(&self.helm_chart, &[]);

        //upgrade
        target
            .helm
            .upgrade(&self.helm_chart, &[], &CommandKiller::from_cancelable(target.should_abort))
            .map_err(|e| EngineError::new_helm_error(self.event_details.clone(), e))
    }

    fn on_pause(&self, _target: &DeploymentTarget) -> Result<(), EngineError> {
        Ok(())
    }

    fn on_delete(&self, target: &DeploymentTarget) -> Result<(), EngineError> {
        target
            .helm
            .uninstall(&self.helm_chart, &[])
            .map_err(|e| EngineError::new_helm_error(self.event_details.clone(), e))?;

        // helm does not wait for pod to terminate https://github.com/helm/helm/issues/10586
        // So wait for
        if let Some(pod_selector) = &self.helm_chart.k8s_selector {
            block_on(async {
                let started = Instant::now();

                let pods: Api<Pod> = Api::namespaced(target.kube.clone(), target.environment.namespace());
                while let Ok(pod) = pods.list(&ListParams::default().labels(pod_selector)).await {
                    if pod.items.is_empty() {
                        break;
                    }

                    if started.elapsed() > DEFAULT_HELM_TIMEOUT {
                        break;
                    }

                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
            });
        }

        Ok(())
    }
}

#[cfg(feature = "test-local-kube")]
#[cfg(test)]
mod tests {
    use crate::cloud_provider::helm::ChartInfo;
    use crate::cmd::helm::Helm;
    use crate::deployment_action::deploy_helm::HelmDeployment;
    use crate::events::{EventDetails, InfrastructureStep, Stage, Transmitter};
    use crate::io_models::QoveryIdentifier;
    use function_name::named;

    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use uuid::Uuid;

    #[test]
    #[named]
    fn test_helm_deployment() -> Result<(), Box<dyn std::error::Error>> {
        let namespace = format!(
            "{}-{:?}",
            function_name!().replace('_', "-"),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()
        );

        let event_details = EventDetails::new(
            None,
            QoveryIdentifier::new_random(),
            QoveryIdentifier::new_random(),
            Uuid::new_v4().to_string(),
            Stage::Infrastructure(InfrastructureStep::RetrieveClusterConfig),
            Transmitter::TaskManager(Uuid::new_v4(), "engine".to_string()),
        );

        let dest_folder = PathBuf::from(format!("/tmp/{}", namespace));
        let chart = ChartInfo::new_from_custom_namespace(
            "test_app_helm_deployment".to_string(),
            dest_folder.to_string_lossy().to_string(),
            namespace,
            600,
            vec![],
            vec![],
            vec![],
            false,
            None,
        );

        let mut tera_context = tera::Context::default();
        tera_context.insert("app_name", "pause");
        let helm = HelmDeployment::new(
            event_details,
            tera_context,
            PathBuf::from("tests/helm/simple_app_deployment"),
            None,
            chart.clone(),
        );

        // Render a simple chart
        helm.prepare_helm_chart().unwrap();

        let mut kube_config = dirs::home_dir().unwrap();
        kube_config.push(".kube/config");
        let helm = Helm::new(kube_config.to_str().unwrap(), &[])?;

        // Check that helm can validate our chart
        helm.template_validate(&chart, &[], None)?;

        Ok(())
    }
}
