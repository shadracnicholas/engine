use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::net::TcpStream;
use std::path::Path;
use std::str::FromStr;

use tera::Context as TeraContext;
use uuid::Uuid;

use crate::cloud_provider::environment::Environment;
use crate::cloud_provider::helm::{ChartInfo, ChartSetValue};
use crate::cloud_provider::kubernetes::{
    kube_copy_secret_to_another_namespace, kube_does_secret_exists, Kind, Kubernetes,
};
use crate::cloud_provider::DeploymentTarget;
use crate::cmd;
use crate::cmd::helm;
use crate::cmd::kubectl::{kubectl_exec_delete_pod, kubectl_exec_delete_secret, kubectl_exec_get_pods};
use crate::cmd::structs::KubernetesPodStatusPhase;
use crate::cmd::terraform::TerraformError;
use crate::errors::{CommandError, EngineError};
use crate::events::{EngineEvent, EventDetails, EventMessage, Stage, ToTransmitter};
use crate::io_models::ProgressLevel::Info;
use crate::io_models::{
    ApplicationAdvancedSettings, Context, DatabaseMode, Listen, ListenersHelper, ProgressInfo, ProgressLevel,
    ProgressScope, QoveryIdentifier,
};
use crate::logger::Logger;
use crate::models::database::DatabaseService;

use crate::models::types::VersionsNumber;
use crate::runtime::block_on;

use super::kubernetes::kube_create_namespace_if_not_exists;

// todo: delete this useless trait
pub trait Service: ToTransmitter {
    fn context(&self) -> &Context;
    fn service_type(&self) -> ServiceType;
    fn id(&self) -> &str;
    fn long_id(&self) -> &Uuid;
    fn name(&self) -> &str;
    fn sanitized_name(&self) -> String;
    fn workspace_directory(&self) -> String {
        let dir_root = match self.service_type() {
            ServiceType::Application => "applications",
            ServiceType::Database(_) => "databases",
            ServiceType::Router => "routers",
        };

        crate::fs::workspace_directory(
            self.context().workspace_root_dir(),
            self.context().execution_id(),
            format!("{}/{}", dir_root, self.name()),
        )
        .unwrap()
    }
    fn get_event_details(&self, stage: Stage) -> EventDetails {
        let context = self.context();
        EventDetails::new(
            None,
            QoveryIdentifier::from(context.organization_id().to_string()),
            QoveryIdentifier::from(context.cluster_id().to_string()),
            QoveryIdentifier::from(context.execution_id().to_string()),
            None,
            stage,
            self.to_transmitter(),
        )
    }
    fn application_advanced_settings(&self) -> Option<ApplicationAdvancedSettings>;
    fn version(&self) -> String;
    fn action(&self) -> &Action;
    fn private_port(&self) -> Option<u16>;
    fn total_cpus(&self) -> String;
    fn cpu_burst(&self) -> String;
    fn total_ram_in_mib(&self) -> u32;
    fn min_instances(&self) -> u32;
    fn max_instances(&self) -> u32;
    fn publicly_accessible(&self) -> bool;
    fn fqdn(&self, target: &DeploymentTarget, fqdn: &str, is_managed: bool) -> String {
        match &self.publicly_accessible() {
            true => fqdn.to_string(),
            false => match is_managed {
                true => format!("{}-dns.{}.svc.cluster.local", self.id(), target.environment.namespace()),
                false => format!("{}.{}.svc.cluster.local", self.sanitized_name(), target.environment.namespace()),
            },
        }
    }
    // used to retrieve logs by using Kubernetes labels (selector)
    fn logger(&self) -> &dyn Logger;
    fn selector(&self) -> Option<String>;
    fn is_listening(&self, ip: &str) -> bool {
        let private_port = match self.private_port() {
            Some(private_port) => private_port,
            _ => return false,
        };

        TcpStream::connect(format!("{}:{}", ip, private_port)).is_ok()
    }

    fn progress_scope(&self) -> ProgressScope {
        let id = self.id().to_string();

        match self.service_type() {
            ServiceType::Application => ProgressScope::Application { id },
            ServiceType::Database(_) => ProgressScope::Database { id },
            ServiceType::Router => ProgressScope::Router { id },
        }
    }

    fn as_service(&self) -> &dyn Service;
}

pub trait Terraform {
    fn terraform_common_resource_dir_path(&self) -> String;
    fn terraform_resource_dir_path(&self) -> String;
}

pub trait Helm {
    fn helm_selector(&self) -> Option<String>;
    fn helm_release_name(&self) -> String;
    fn helm_chart_dir(&self) -> String;
    fn helm_chart_values_dir(&self) -> String;
    fn helm_chart_external_name_service_dir(&self) -> String;
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub enum Action {
    Create,
    Pause,
    Delete,
    Nothing,
}

#[derive(Eq, PartialEq)]
pub struct DatabaseOptions {
    pub login: String,
    pub password: String,
    pub host: String,
    pub port: u16,
    pub mode: DatabaseMode,
    pub disk_size_in_gib: u32,
    pub database_disk_type: String,
    pub encrypt_disk: bool,
    pub activate_high_availability: bool,
    pub activate_backups: bool,
    pub publicly_accessible: bool,
}

#[derive(Debug, Eq, PartialEq, Copy, Clone, Serialize)]
pub enum DatabaseType {
    PostgreSQL,
    MongoDB,
    MySQL,
    Redis,
}

impl ToString for DatabaseType {
    fn to_string(&self) -> String {
        match self {
            DatabaseType::PostgreSQL => "PostgreSQL".to_string(),
            DatabaseType::MongoDB => "MongoDB".to_string(),
            DatabaseType::MySQL => "MySQL".to_string(),
            DatabaseType::Redis => "Redis".to_string(),
        }
    }
}

#[derive(Eq, PartialEq)]
pub enum ServiceType {
    Application,
    Database(DatabaseType),
    Router,
}

impl ServiceType {
    pub fn name(&self) -> String {
        match self {
            ServiceType::Application => "Application".to_string(),
            ServiceType::Database(db_type) => format!("{} database", db_type.to_string()),
            ServiceType::Router => "Router".to_string(),
        }
    }
}

impl ToString for ServiceType {
    fn to_string(&self) -> String {
        self.name()
    }
}

pub fn default_tera_context(
    service: &dyn Service,
    kubernetes: &dyn Kubernetes,
    environment: &Environment,
) -> TeraContext {
    let mut context = TeraContext::new();
    context.insert("id", service.id());
    context.insert("long_id", service.long_id());
    context.insert("owner_id", environment.owner_id.as_str());
    context.insert("project_id", environment.project_id.as_str());
    context.insert("project_long_id", &environment.project_long_id);
    context.insert("organization_id", environment.organization_id.as_str());
    context.insert("organization_long_id", &environment.organization_long_id);
    context.insert("environment_id", environment.id.as_str());
    context.insert("environment_long_id", &environment.long_id);
    context.insert("region", kubernetes.region().as_str());
    context.insert("zone", kubernetes.zone());
    context.insert("name", service.name());
    context.insert("sanitized_name", &service.sanitized_name());
    context.insert("namespace", environment.namespace());
    context.insert("cluster_name", kubernetes.name());
    context.insert("total_cpus", &service.total_cpus());
    context.insert("total_ram_in_mib", &service.total_ram_in_mib());
    context.insert("min_instances", &service.min_instances());
    context.insert("max_instances", &service.max_instances());

    context.insert("is_private_port", &service.private_port().is_some());
    if let Some(private_port) = service.private_port() {
        context.insert("private_port", &private_port);
    }

    context.insert("version", &service.version());

    context
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseTerraformConfig {
    #[serde(rename = "database_target_id")]
    pub target_id: String,
    #[serde(rename = "database_target_hostname")]
    pub target_hostname: String,
    #[serde(rename = "database_target_fqdn_id")]
    pub target_fqdn_id: String,
    #[serde(rename = "database_target_fqdn")]
    pub target_fqdn: String,
}

pub fn get_database_terraform_config(
    database_terraform_config_file: &str,
) -> Result<DatabaseTerraformConfig, TerraformError> {
    let file_content = match File::open(&database_terraform_config_file) {
        Ok(f) => f,
        Err(e) => {
            return Err(TerraformError::ConfigFileNotFound {
                path: database_terraform_config_file.to_string(),
                raw_message: format!("Terraform config error, database config cannot be found.\n{}", e),
            });
        }
    };

    let reader = BufReader::new(file_content);
    match serde_json::from_reader(reader) {
        Ok(config) => Ok(config),
        Err(e) => Err(TerraformError::ConfigFileInvalidContent {
            path: database_terraform_config_file.to_string(),
            raw_message: format!("Terraform config error, database config cannot be parsed.\n{}", e),
        }),
    }
}

pub fn prepare_namespace(
    environment: &Environment,
    namespace_labels: Option<BTreeMap<String, String>>,
    event_details: EventDetails,
    kubernetes_kind: Kind,
    kube: &kube::Client,
) -> Result<(), EngineError> {
    // create a namespace with labels if it does not exist
    block_on(kube_create_namespace_if_not_exists(
        kube,
        environment.namespace(),
        namespace_labels,
    ))
    .map_err(|e| {
        EngineError::new_k8s_create_namespace(
            event_details.clone(),
            environment.namespace().to_string(),
            CommandError::new(
                format!("Can't create namespace {}", environment.namespace()),
                Some(e.to_string()),
                None,
            ),
        )
    })?;

    // upmc-enterprises/registry-creds sometimes is too long to copy the secret to the namespace
    // this workaround speed up the process to avoid application fails with ImagePullError on the first deployment
    if kubernetes_kind == Kind::Ec2 {
        let from_namespace = "default";
        match block_on(kube_does_secret_exists(kube, "awsecr-cred", "default")) {
            Ok(x) if x => {
                block_on(kube_copy_secret_to_another_namespace(
                    kube,
                    "awsecr-cred",
                    from_namespace,
                    environment.namespace(),
                ))
                .map_err(|e| {
                    EngineError::new_copy_secrets_to_another_namespace_error(
                        event_details.clone(),
                        e,
                        from_namespace,
                        environment.namespace(),
                    )
                })?;
            }
            _ => {}
        };
    };

    Ok(())
}

pub fn deploy_managed_database_service<T>(
    target: &DeploymentTarget,
    service: &T,
    event_details: EventDetails,
) -> Result<(), EngineError>
where
    T: DatabaseService + Helm + Terraform,
{
    let workspace_dir = service.workspace_directory();
    let kubernetes = target.kubernetes;
    let environment = target.environment;

    let _context = service.to_tera_context(target)?;
    let kubernetes_config_file_path = kubernetes.get_kubeconfig_file_path()?;

    // define labels to add to namespace
    let mut namespace_labels: Option<BTreeMap<String, String>> = None;
    if service.context().resource_expiration_in_seconds().is_some() {
        namespace_labels = Some(BTreeMap::from([(
            "ttl".to_string(),
            format!(
                "{}",
                service
                    .context()
                    .resource_expiration_in_seconds()
                    .expect("expected to have resource expiration in seconds")
            ),
        )]));
    };

    prepare_namespace(
        environment,
        namespace_labels,
        event_details.clone(),
        kubernetes.kind(),
        &target.kube,
    )?;

    // do exec helm upgrade and return the last deployment status
    let helm = helm::Helm::new(
        &kubernetes_config_file_path,
        &kubernetes.cloud_provider().credentials_environment_variables(),
    )
    .map_err(|e| helm::to_engine_error(&event_details, e))?;

    let context = service.to_tera_context(target)?;

    if let Err(e) = crate::template::generate_and_copy_all_files_into_dir(
        service.terraform_common_resource_dir_path(),
        &workspace_dir,
        context.clone(),
    ) {
        return Err(EngineError::new_cannot_copy_files_from_one_directory_to_another(
            event_details,
            service.terraform_common_resource_dir_path(),
            workspace_dir,
            e,
        ));
    }

    if let Err(e) = crate::template::generate_and_copy_all_files_into_dir(
        service.terraform_resource_dir_path(),
        &workspace_dir,
        context.clone(),
    ) {
        return Err(EngineError::new_cannot_copy_files_from_one_directory_to_another(
            event_details,
            service.terraform_resource_dir_path(),
            workspace_dir,
            e,
        ));
    }

    let external_svc_dir = format!("{}/{}", workspace_dir, "external-name-svc");
    if let Err(e) = crate::template::generate_and_copy_all_files_into_dir(
        service.helm_chart_external_name_service_dir(),
        external_svc_dir.as_str(),
        context,
    ) {
        return Err(EngineError::new_cannot_copy_files_from_one_directory_to_another(
            event_details,
            service.helm_chart_external_name_service_dir(),
            external_svc_dir,
            e,
        ));
    }

    cmd::terraform::terraform_init_validate_plan_apply(workspace_dir.as_str(), service.context().is_dry_run_deploy())
        .map_err(|e| EngineError::new_terraform_error(event_details.clone(), e))?;

    // Gather database TF generated JSON config if any
    // if configuration exists, it means that HELM will be used to deploy managed DB service external name chart instead on Terraform
    match get_database_terraform_config(format!("{}/database-tf-config.json", workspace_dir).as_str()) {
        Ok(database_config) => {
            // Deploying helm chart
            let chart = ChartInfo::new_from_custom_namespace(
                format!("{}-externalname", database_config.target_id),
                external_svc_dir,
                environment.namespace().to_string(),
                600_i64,
                vec![],
                vec![
                    ChartSetValue {
                        key: "target_hostname".to_string(),
                        value: database_config.target_hostname,
                    },
                    ChartSetValue {
                        key: "source_fqdn".to_string(),
                        value: database_config.target_fqdn,
                    },
                    ChartSetValue {
                        key: "database_id".to_string(),
                        value: service.id().to_string(),
                    },
                    ChartSetValue {
                        key: "database_long_id".to_string(),
                        value: service.long_id().to_string(),
                    },
                    ChartSetValue {
                        key: "environment_id".to_string(),
                        value: environment.id.to_string(),
                    },
                    ChartSetValue {
                        key: "environment_long_id".to_string(),
                        value: environment.long_id.to_string(),
                    },
                    ChartSetValue {
                        key: "project_long_id".to_string(),
                        value: environment.project_long_id.to_string(),
                    },
                    ChartSetValue {
                        key: "service_name".to_string(),
                        value: database_config.target_fqdn_id,
                    },
                    ChartSetValue {
                        key: "publicly_accessible".to_string(),
                        value: service.publicly_accessible().to_string(),
                    },
                ],
                vec![],
                false,
                service.selector(),
            );

            helm.upgrade(&chart, &[])
                .map_err(|e| helm::to_engine_error(&event_details, e))?;
        }
        Err(e) => return Err(EngineError::new_terraform_error(event_details, e)),
    }

    Ok(())
}

pub fn delete_managed_stateful_service<T>(
    target: &DeploymentTarget,
    service: &T,
    event_details: EventDetails,
    logger: &dyn Logger,
) -> Result<(), EngineError>
where
    T: DatabaseService + Helm + Terraform,
{
    assert!(
        service.is_managed_service(),
        "trying to deploy a service that is not managed as a managed one"
    );

    let kubernetes = target.kubernetes;
    let environment = target.environment;
    let workspace_dir = service.workspace_directory();
    let tera_context = service.to_tera_context(target)?;

    if let Err(e) = crate::template::generate_and_copy_all_files_into_dir(
        service.terraform_common_resource_dir_path(),
        workspace_dir.as_str(),
        tera_context.clone(),
    ) {
        return Err(EngineError::new_cannot_copy_files_from_one_directory_to_another(
            event_details,
            service.terraform_common_resource_dir_path(),
            workspace_dir,
            e,
        ));
    }

    if let Err(e) = crate::template::generate_and_copy_all_files_into_dir(
        service.terraform_resource_dir_path(),
        workspace_dir.as_str(),
        tera_context.clone(),
    ) {
        return Err(EngineError::new_cannot_copy_files_from_one_directory_to_another(
            event_details,
            service.terraform_resource_dir_path(),
            workspace_dir,
            e,
        ));
    }

    let external_svc_dir = format!("{}/{}", workspace_dir, "external-name-svc");
    if let Err(e) = crate::template::generate_and_copy_all_files_into_dir(
        service.helm_chart_external_name_service_dir(),
        &external_svc_dir,
        tera_context.clone(),
    ) {
        return Err(EngineError::new_cannot_copy_files_from_one_directory_to_another(
            event_details,
            service.helm_chart_external_name_service_dir(),
            external_svc_dir,
            e,
        ));
    }

    if let Err(e) = crate::template::generate_and_copy_all_files_into_dir(
        service.helm_chart_external_name_service_dir(),
        workspace_dir.as_str(),
        tera_context,
    ) {
        return Err(EngineError::new_cannot_copy_files_from_one_directory_to_another(
            event_details,
            service.helm_chart_external_name_service_dir(),
            workspace_dir,
            e,
        ));
    }

    match cmd::terraform::terraform_init_validate_destroy(workspace_dir.as_str(), true) {
        Ok(_) => {
            logger.log(EngineEvent::Info(
                event_details,
                EventMessage::new_from_safe("Deleting secret containing tfstates".to_string()),
            ));
            let _ = delete_terraform_tfstate_secret(kubernetes, environment.namespace(), &get_tfstate_name(service));
        }
        Err(e) => {
            let engine_err = EngineError::new_terraform_error(event_details, e);

            logger.log(EngineEvent::Error(engine_err.clone(), None));

            return Err(engine_err);
        }
    }

    Ok(())
}

pub struct ServiceVersionCheckResult {
    requested_version: VersionsNumber,
    matched_version: VersionsNumber,
    message: Option<String>,
}

impl ServiceVersionCheckResult {
    pub fn new(requested_version: VersionsNumber, matched_version: VersionsNumber, message: Option<String>) -> Self {
        ServiceVersionCheckResult {
            requested_version,
            matched_version,
            message,
        }
    }

    pub fn matched_version(&self) -> VersionsNumber {
        self.matched_version.clone()
    }

    pub fn requested_version(&self) -> &VersionsNumber {
        &self.requested_version
    }

    pub fn message(&self) -> Option<String> {
        self.message.clone()
    }
}

pub fn check_service_version<T>(
    result: Result<String, CommandError>,
    service: &T,
    event_details: EventDetails,
    logger: &dyn Logger,
) -> Result<ServiceVersionCheckResult, EngineError>
where
    T: Service + Listen,
{
    let listeners_helper = ListenersHelper::new(service.listeners());

    match result {
        Ok(version) => {
            if service.version() != version.as_str() {
                let message = format!(
                    "{} version `{}` has been requested by the user; but matching version is `{}`",
                    service.service_type().name(),
                    service.version(),
                    version.as_str()
                );

                logger.log(EngineEvent::Info(
                    event_details.clone(),
                    EventMessage::new_from_safe(message.to_string()),
                ));

                let progress_info = ProgressInfo::new(
                    service.progress_scope(),
                    Info,
                    Some(message.to_string()),
                    service.context().execution_id(),
                );

                listeners_helper.deployment_in_progress(progress_info);

                return Ok(ServiceVersionCheckResult::new(
                    VersionsNumber::from_str(&service.version()).map_err(|e| {
                        EngineError::new_version_number_parsing_error(event_details.clone(), service.version(), e)
                    })?,
                    VersionsNumber::from_str(&version).map_err(|e| {
                        EngineError::new_version_number_parsing_error(event_details.clone(), version.to_string(), e)
                    })?,
                    Some(message),
                ));
            }

            Ok(ServiceVersionCheckResult::new(
                VersionsNumber::from_str(&service.version()).map_err(|e| {
                    EngineError::new_version_number_parsing_error(event_details.clone(), service.version(), e)
                })?,
                VersionsNumber::from_str(&version).map_err(|e| {
                    EngineError::new_version_number_parsing_error(event_details.clone(), version.to_string(), e)
                })?,
                None,
            ))
        }
        Err(_err) => {
            let message = format!(
                "{} version {} is not supported!",
                service.service_type().name(),
                service.version(),
            );

            let progress_info = ProgressInfo::new(
                service.progress_scope(),
                ProgressLevel::Error,
                Some(message),
                service.context().execution_id(),
            );

            listeners_helper.deployment_error(progress_info);

            let error = EngineError::new_unsupported_version_error(
                event_details,
                service.service_type().name(),
                service.version(),
            );

            logger.log(EngineEvent::Error(error.clone(), None));

            Err(error)
        }
    }
}

fn delete_terraform_tfstate_secret(
    kubernetes: &dyn Kubernetes,
    namespace: &str,
    secret_name: &str,
) -> Result<(), EngineError> {
    let config_file_path = kubernetes.get_kubeconfig_file_path()?;

    // create the namespace to insert the tfstate in secrets
    let _ = kubectl_exec_delete_secret(
        config_file_path,
        namespace,
        secret_name,
        kubernetes.cloud_provider().credentials_environment_variables(),
    );

    Ok(())
}

pub enum CheckAction {
    Deploy,
    Pause,
    Delete,
}

pub fn helm_uninstall_release(
    kubernetes: &dyn Kubernetes,
    environment: &Environment,
    helm_release_name: &str,
    event_details: EventDetails,
) -> Result<(), EngineError> {
    let kubernetes_config_file_path = kubernetes.get_kubeconfig_file_path()?;

    let helm = helm::Helm::new(
        &kubernetes_config_file_path,
        &kubernetes.cloud_provider().credentials_environment_variables(),
    )
    .map_err(|e| EngineError::new_helm_error(event_details.clone(), e))?;

    let chart = ChartInfo::new_from_release_name(helm_release_name, environment.namespace());
    helm.uninstall(&chart, &[])
        .map_err(|e| EngineError::new_helm_error(event_details.clone(), e))
}

pub fn get_tfstate_suffix(service: &dyn Service) -> String {
    service.id().to_string()
}

// Name generated from TF secret suffix
// https://www.terraform.io/docs/backends/types/kubernetes.html#secret_suffix
// As mention the doc: Secrets will be named in the format: tfstate-{workspace}-{secret_suffix}.
pub fn get_tfstate_name(service: &dyn Service) -> String {
    format!("tfstate-default-{}", service.id())
}

pub fn delete_pending_service<P>(
    kubernetes_config: P,
    namespace: &str,
    selector: &str,
    envs: Vec<(&str, &str)>,
    event_details: EventDetails,
) -> Result<(), EngineError>
where
    P: AsRef<Path>,
{
    match kubectl_exec_get_pods(&kubernetes_config, Some(namespace), Some(selector), envs.clone()) {
        Ok(pods) => {
            for pod in pods.items {
                if pod.status.phase == KubernetesPodStatusPhase::Pending {
                    if let Err(e) = kubectl_exec_delete_pod(
                        &kubernetes_config,
                        pod.metadata.namespace.as_str(),
                        pod.metadata.name.as_str(),
                        envs.clone(),
                    ) {
                        return Err(EngineError::new_k8s_service_issue(event_details, e));
                    }
                }
            }

            Ok(())
        }
        Err(e) => Err(EngineError::new_k8s_service_issue(event_details, e)),
    }
}
