pub mod io;

extern crate derivative;
extern crate url;

use crate::build_platform::BuildError;
use crate::cloud_provider::Kind;
use crate::cmd;
use crate::cmd::docker::DockerError;
use crate::cmd::helm::HelmError;
use crate::cmd::terraform::{QuotaExceededError, TerraformError};
use crate::container_registry::errors::ContainerRegistryError;

use crate::cmd::command;
use crate::events::{EventDetails, Stage};
use crate::models::types::VersionsNumber;
use crate::object_storage::errors::ObjectStorageError;
use derivative::Derivative;
use kube::error::Error as KubeError;
use std::fmt::{Display, Formatter};
use std::io::Error;
use thiserror::Error;
use url::Url;

const DEFAULT_HINT_MESSAGE: &str = "Need Help ? Please consult our FAQ to troubleshoot your deployment https://hub.qovery.com/docs/using-qovery/troubleshoot/ and visit the forum https://discuss.qovery.com/";

/// ErrorMessageVerbosity: represents command error message's verbosity from minimal to full verbosity.
pub enum ErrorMessageVerbosity {
    SafeOnly,
    FullDetailsWithoutEnvVars,
    FullDetails,
}

/// CommandError: command error, mostly returned by third party tools.
#[derive(Derivative, Clone, Error, PartialEq, Eq)]
#[derivative(Debug)]
pub struct CommandError {
    /// full_details: full error message, can contains unsafe text such as passwords and tokens.
    full_details: Option<String>,
    /// message_safe: error message omitting displaying any protected data such as passwords and tokens.
    message_safe: String,
    /// env_vars: environments variables including touchy data such as secret keys.
    /// env_vars field is ignored from any wild Debug printing because of it touchy data it carries.
    #[derivative(Debug = "ignore")]
    env_vars: Option<Vec<(String, String)>>,
}

impl From<command::CommandError> for CommandError {
    fn from(err: command::CommandError) -> Self {
        CommandError::new(err.to_string(), None, None)
    }
}

impl CommandError {
    /// Returns CommandError message_raw. May contains unsafe text such as passwords and tokens.
    pub fn message_raw(&self) -> Option<String> {
        self.full_details.clone()
    }

    /// Returns CommandError message_safe omitting all unsafe text such as passwords and tokens.
    pub fn message_safe(&self) -> String {
        self.message_safe.to_string()
    }

    /// Returns CommandError env_vars.
    pub fn env_vars(&self) -> Option<Vec<(String, String)>> {
        self.env_vars.clone()
    }

    /// Returns error message based on verbosity.
    pub fn message(&self, message_verbosity: ErrorMessageVerbosity) -> String {
        match message_verbosity {
            ErrorMessageVerbosity::SafeOnly => self.message_safe.to_string(),
            ErrorMessageVerbosity::FullDetailsWithoutEnvVars => match &self.full_details {
                None => self.message(ErrorMessageVerbosity::SafeOnly),
                Some(full_details) => format!("{} / Full details: {}", self.message_safe, full_details),
            },
            ErrorMessageVerbosity::FullDetails => match &self.full_details {
                None => self.message(ErrorMessageVerbosity::SafeOnly),
                Some(full_details) => match &self.env_vars {
                    None => format!("{} / Full details: {}", self.message_safe, full_details),
                    Some(env_vars) => {
                        format!(
                            "{} / Full details: {} / Env vars: {}",
                            self.message_safe,
                            full_details,
                            env_vars
                                .iter()
                                .map(|(k, v)| format!("{}={}", k, v))
                                .collect::<Vec<String>>()
                                .join(" "),
                        )
                    }
                },
            },
        }
    }

    /// Creates a new CommandError from safe message. To be used when message is safe.
    pub fn new_from_safe_message(message: String) -> Self {
        CommandError::new(message, None, None)
    }

    /// Creates a new CommandError having both a safe, an unsafe message and env vars.
    pub fn new(message_safe: String, message_raw: Option<String>, env_vars: Option<Vec<(String, String)>>) -> Self {
        CommandError {
            full_details: message_raw,
            message_safe,
            env_vars,
        }
    }

    /// Creates a new CommandError from legacy command error.
    pub fn new_from_legacy_command_error(
        legacy_command_error: cmd::command::CommandError,
        safe_message: Option<String>,
    ) -> Self {
        CommandError {
            full_details: Some(legacy_command_error.to_string()),
            message_safe: safe_message.unwrap_or_else(|| "No message".to_string()),
            env_vars: None,
        }
    }

    /// Create a new CommandError from a CMD command.
    pub fn new_from_command_line(
        message: String,
        bin: String,
        cmd_args: Vec<String>,
        envs: Vec<(String, String)>,
        stdout: Option<String>,
        stderr: Option<String>,
    ) -> Self {
        let mut unsafe_message = format!("{}\ncommand: {} {}", message, bin, cmd_args.join(" "),);

        if let Some(txt) = stdout {
            unsafe_message = format!("{}\nSTDOUT {}", unsafe_message, txt);
        }
        if let Some(txt) = stderr {
            unsafe_message = format!("{}\nSTDERR {}", unsafe_message, txt);
        }

        CommandError::new(message, Some(unsafe_message), Some(envs))
    }
}

impl Default for CommandError {
    fn default() -> Self {
        Self {
            full_details: None,
            message_safe: "Unknown command error".to_string(),
            env_vars: None,
        }
    }
}

impl Display for CommandError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message(ErrorMessageVerbosity::SafeOnly).as_str()) // By default, expose safe message only
    }
}

impl From<serde_json::Error> for CommandError {
    fn from(err: serde_json::Error) -> Self {
        CommandError::new(
            "Serde error while performing Serialization / Deserialization".to_string(),
            Some(err.to_string()),
            None,
        )
    }
}

impl From<Error> for CommandError {
    fn from(err: Error) -> Self {
        CommandError::new("IO error".to_string(), Some(err.to_string()), None)
    }
}

impl From<ObjectStorageError> for CommandError {
    fn from(object_storage_error: ObjectStorageError) -> Self {
        // Note: safe message to be manually computed here because we are not 100% sure error won't leak some data
        match object_storage_error {
            ObjectStorageError::InvalidBucketName {
                bucket_name,
                raw_error_message,
            } => CommandError::new(
                format!("Object storage error, invalid bucket name: `{}`", bucket_name),
                Some(raw_error_message),
                None,
            ),
            ObjectStorageError::CannotCreateBucket {
                bucket_name,
                raw_error_message,
            } => CommandError::new(
                format!("Object storage error, cannot create bucket: `{}`", bucket_name),
                Some(raw_error_message),
                None,
            ),
            ObjectStorageError::CannotDeleteBucket {
                bucket_name,
                raw_error_message,
            } => CommandError::new(
                format!("Object storage error, cannot delete bucket: `{}`", bucket_name),
                Some(raw_error_message),
                None,
            ),
            ObjectStorageError::CannotEmptyBucket {
                bucket_name,
                raw_error_message,
            } => CommandError::new(
                format!("Object storage error, cannot empty bucket: `{}`", bucket_name),
                Some(raw_error_message),
                None,
            ),
            ObjectStorageError::CannotTagBucket {
                bucket_name,
                raw_error_message,
            } => CommandError::new(
                format!("Object storage error, cannot tag bucket: `{}`", bucket_name),
                Some(raw_error_message),
                None,
            ),
            ObjectStorageError::CannotGetObjectFile {
                bucket_name,
                file_name,
                raw_error_message,
            } => CommandError::new(
                format!(
                    "Object storage error, cannot get file: `{}` in bucket: `{}`",
                    file_name, bucket_name
                ),
                Some(raw_error_message),
                None,
            ),
            ObjectStorageError::CannotDeleteFile {
                bucket_name,
                file_name,
                raw_error_message,
            } => CommandError::new(
                format!(
                    "Object storage error, cannot delete file `{}` from bucket: `{}`",
                    file_name, bucket_name
                ),
                Some(raw_error_message),
                None,
            ),
            ObjectStorageError::QuotasExceeded {
                bucket_name,
                raw_error_message,
            } => CommandError::new(
                format!("Object storage error, quotas exceeded: `{}`", bucket_name),
                Some(raw_error_message),
                None,
            ),
            ObjectStorageError::CannotActivateBucketVersioning {
                bucket_name,
                raw_error_message,
            } => CommandError::new(
                format!("Object storage error, cannot activate bucket versioning for: `{}`", bucket_name),
                Some(raw_error_message),
                None,
            ),
            ObjectStorageError::CannotUploadFile {
                bucket_name,
                file_name,
                raw_error_message,
            } => CommandError::new(
                format!(
                    "Object storage error, cannot upload file `{}` into bucket: `{}`",
                    file_name, bucket_name
                ),
                Some(raw_error_message),
                None,
            ),
        }
    }
}

impl From<ContainerRegistryError> for CommandError {
    fn from(container_registry_error: ContainerRegistryError) -> Self {
        // Note: safe message to be manually computed here because we are not 100% sure error won't leak some data
        match container_registry_error {
            ContainerRegistryError::InvalidCredentials => {
                CommandError::new_from_safe_message("Container registry error, invalid credentials".to_string())
            }
            ContainerRegistryError::CannotGetCredentials => {
                CommandError::new_from_safe_message("Container registry error, cannot get credentials".to_string())
            }
            ContainerRegistryError::CannotCreateRegistry {
                registry_name,
                raw_error_message,
            } => CommandError::new(
                format!("Container registry error, cannot create registry: `{}`", registry_name),
                Some(raw_error_message),
                None,
            ),
            ContainerRegistryError::CannotDeleteRegistry {
                registry_name,
                raw_error_message,
            } => CommandError::new(
                format!("Container registry error, cannot delete registry: `{}`", registry_name),
                Some(raw_error_message),
                None,
            ),
            ContainerRegistryError::CannotDeleteImage {
                registry_name,
                repository_name,
                image_name,
                raw_error_message,
            } => CommandError::new(
                format!(
                    "Container registry error, cannot delete image `{}` from repository: `{}` in registry: `{}`",
                    image_name, repository_name, registry_name
                ),
                Some(raw_error_message),
                None,
            ),
            ContainerRegistryError::ImageDoesntExistInRegistry {
                registry_name,
                repository_name,
                image_name,
            } => CommandError::new_from_safe_message(format!(
                "Container registry error, image `{}` doesn't exists in repository `{}` in registry: `{}`",
                image_name, repository_name, registry_name
            )),
            ContainerRegistryError::RepositoryDoesntExistInRegistry {
                registry_name,
                repository_name,
            } => CommandError::new_from_safe_message(format!(
                "Container registry error, repository `{}` doesn't exist in registry: `{}`",
                repository_name, registry_name
            )),
            ContainerRegistryError::RegistryDoesntExist {
                registry_name,
                raw_error_message,
            } => CommandError::new(
                format!("Container registry error, registry: `{}` doesn't exist", registry_name),
                Some(raw_error_message),
                None,
            ),
            ContainerRegistryError::CannotLinkRegistryToCluster {
                registry_name,
                cluster_id,
                raw_error_message,
            } => CommandError::new(
                format!(
                    "Container registry error, cannot link cluster with id `{}` to registry: `{}`",
                    cluster_id, registry_name
                ),
                Some(raw_error_message),
                None,
            ),
            ContainerRegistryError::CannotCreateRepository {
                repository_name,
                registry_name,
                raw_error_message,
            } => CommandError::new(
                format!(
                    "Container registry error, cannot create repository `{}` in registry: `{}`",
                    repository_name, registry_name
                ),
                Some(raw_error_message),
                None,
            ),
            ContainerRegistryError::CannotDeleteRepository {
                repository_name,
                registry_name,
                raw_error_message,
            } => CommandError::new(
                format!(
                    "Container registry error, cannot delete repository `{}` in registry: `{}`",
                    repository_name, registry_name
                ),
                Some(raw_error_message),
                None,
            ),
            ContainerRegistryError::CannotSetRepositoryLifecyclePolicy {
                repository_name,
                registry_name,
                raw_error_message,
            } => CommandError::new(
                format!(
                    "Container registry error, cannot set lifetime policy for repository `{}` in registry: `{}`",
                    repository_name, registry_name
                ),
                Some(raw_error_message),
                None,
            ),
            ContainerRegistryError::CannotSetRepositoryTags {
                registry_name,
                repository_name,
                raw_error_message,
            } => CommandError::new(
                format!(
                    "Container registry error, cannot set tags for for repository `{}` in registry: `{}`",
                    repository_name, registry_name
                ),
                Some(raw_error_message),
                None,
            ),
            ContainerRegistryError::Unknown { raw_error_message } => {
                CommandError::new("Container registry unknown error.".to_string(), Some(raw_error_message), None)
            }
        }
    }
}

impl From<BuildError> for CommandError {
    fn from(build_error: BuildError) -> Self {
        // Note: safe message to be manually computed here because we are not 100% sure error won't leak some data
        match build_error {
            BuildError::InvalidConfig {
                application,
                raw_error_message,
            } => CommandError::new(
                format!(
                    "Build error, cannot build application `{}` due to an invalid configuration",
                    application
                ),
                Some(raw_error_message),
                None,
            ),
            BuildError::GitError { application, raw_error } => CommandError::new(
                format!("Build error, cannot build application `{}` due to a git error", application),
                Some(raw_error.to_string()),
                None,
            ),
            BuildError::Aborted { application } => CommandError::new_from_safe_message(format!(
                "Build error, application `{}` build has been aborted",
                application
            )),
            BuildError::IoError {
                application,
                action_description,
                raw_error,
            } => CommandError::new(
                format!(
                    "Build error, cannot build application `{}` due to an IO error `{}`",
                    application, action_description
                ),
                Some(raw_error.to_string()),
                None,
            ),
            BuildError::DockerError { application, raw_error } => CommandError::new(
                format!("Build error, cannot build application `{}` due to a Docker error", application),
                Some(raw_error.to_string()),
                None,
            ),
            BuildError::BuildpackError { application, raw_error } => CommandError::new(
                format!(
                    "Build error, cannot build application `{}` due to a Buildpack error",
                    application
                ),
                Some(raw_error.to_string()),
                None,
            ),
        }
    }
}

impl From<DockerError> for CommandError {
    fn from(docker_error: DockerError) -> Self {
        // Note: safe message to be manually computed here because we are not 100% sure error won't leak some data
        match docker_error {
            DockerError::InvalidConfig { raw_error_message } => {
                CommandError::new("Docker error, invalid configuration".to_string(), Some(raw_error_message), None)
            }
            DockerError::ExecutionError { raw_error } => CommandError::new(
                "Docker error, docker terminated with an unknown error".to_string(),
                Some(raw_error.to_string()),
                None,
            ),
            DockerError::ExitStatusError { exit_status } => CommandError::new_from_safe_message(format!(
                "Docker error, docker terminated with a non success exit status: `{}`",
                exit_status
            )),
            DockerError::Aborted { raw_error_message } => CommandError::new(
                "Docker error, aborted due to user cancel request".to_string(),
                Some(raw_error_message),
                None,
            ),
            DockerError::Timeout { raw_error_message } => CommandError::new(
                "Docker error, terminated due to timeout".to_string(),
                Some(raw_error_message),
                None,
            ),
        }
    }
}

impl From<TerraformError> for CommandError {
    fn from(terraform_error: TerraformError) -> Self {
        // Terraform errors are 99% safe and carry useful data to be sent to end-users.
        // Hence, for the time being, everything is sent back to end-users.
        // TODO(benjaminch): Terraform errors should be parsed properly extracting useful data from it.
        CommandError::new(terraform_error.to_string(), None, None)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Tag: unique identifier for an error.
pub enum Tag {
    /// Unknown: unknown error.
    Unknown,
    /// InvalidEnginePayload: represents an error when the received payload contains invalid informations.
    InvalidEnginePayload,
    /// InvalidEngineApiInput: represents an error where Engine's API input is not valid and cannot be deserialized.
    InvalidEngineApiInputCannotBeDeserialized,
    /// MissingRequiredEnvVariable: represents an error where a required env variable is not set.
    MissingRequiredEnvVariable,
    /// NoClusterFound: represents an error where no cluster was found
    NoClusterFound,
    /// ClusterHasNoWorkerNodes: represents an error where the current cluster doesn't have any worker nodes.
    ClusterHasNoWorkerNodes,
    /// ClusterHasNoWorkerNodes: represents an error where the current cluster doesn't have any worker nodes.
    ClusterWorkerNodeNotFound,
    /// CannotGetWorkspaceDirectory: represents an error while trying to get workspace directory.
    CannotGetWorkspaceDirectory,
    /// UnsupportedInstanceType: represents an unsupported instance type for the given cloud provider.
    UnsupportedInstanceType,
    /// NotAllowedInstanceType: represents not allowed instance type for a specific kind of cluster
    NotAllowedInstanceType,
    /// UnsupportedClusterKind: represents an unsupported cluster kind by Qovery.
    UnsupportedClusterKind,
    /// UnsupportedRegion: represents an unsupported region for the given cloud provider.
    UnsupportedRegion,
    /// UnsupportedZone: represents an unsupported zone in region for the given cloud provider.
    UnsupportedZone,
    /// CannotRetrieveKubernetesConfigFile: represents an error while trying to retrieve Kubernetes config file.
    CannotRetrieveClusterConfigFile,
    /// CannotCreateFile: represents an error while trying to create a file.
    CannotCreateFile,
    /// CannotGetClusterNodes: represents an error while trying to get cluster's nodes.
    CannotGetClusterNodes,
    /// NotEnoughNodesAvailableToDeployEnvironment: represents an error when trying to deploy an environment but there the desired number of nodes exceeds the maximum value.
    NotEnoughNodesAvailableToDeployEnvironment,
    /// NotEnoughResourcesToDeployEnvironment: represents an error when trying to deploy an environment but there are not enough resources available on the cluster.
    NotEnoughResourcesToDeployEnvironment,
    /// CannotUninstallHelmChart: represents an error when trying to uninstall an helm chart on the cluster, uninstallation couldn't be proceeded.
    CannotUninstallHelmChart,
    /// CannotExecuteK8sVersion: represents an error when trying to execute kubernetes version command.
    CannotExecuteK8sVersion,
    /// CannotDetermineK8sMasterVersion: represents an error when trying to determine kubernetes master version which cannot be retrieved.
    CannotDetermineK8sMasterVersion,
    /// CannotDetermineK8sRequestedUpgradeVersion: represents an error when trying to determines upgrade requested kubernetes version.
    CannotDetermineK8sRequestedUpgradeVersion,
    /// CannotDetermineK8sKubeletWorkerVersion: represents an error when trying to determine kubelet worker version which cannot be retrieved.
    CannotDetermineK8sKubeletWorkerVersion,
    /// CannotGetNodeGroupList: represents an error while getting node group list from the cloud provider
    CannotGetNodeGroupList,
    /// CannotGetNodeGroupInfo: represent and error caused by the cloud provider because no Nodegroup information has been returned
    CannotGetNodeGroupInfo,
    /// NumberOfMaxNodesIsBelowThanCurrentUsage: represents an error explaining to the user the requested maximum of nodes is below the current usage
    NumberOfRequestedMaxNodesIsBelowThanCurrentUsage,
    /// CannotDetermineK8sKubeProxyVersion: represents an error when trying to determine kube proxy version which cannot be retrieved.
    CannotDetermineK8sKubeProxyVersion,
    /// CannotPauseManagedDatabase: as the title says
    CannotPauseManagedDatabase,
    /// CannotConnectK8sCluster: represents an error when trying to connect to the kubernetes cluster
    CannotConnectK8sCluster,
    /// CannotExecuteK8sApiCustomMetrics: represents an error when trying to get K8s API custom metrics.
    CannotExecuteK8sApiCustomMetrics,
    /// CloudProviderGetLoadBalancer: represents an issue while trying to get load balancers from the cloud provider API
    CloudProviderGetLoadBalancer,
    /// CloudProviderGetLoadBalancerTags: represents an issue while trying to get load balancer tags from the cloud provider API
    CloudProviderGetLoadBalancerTags,
    /// CloudProviderDeleteLoadBalancer: represents an issue while trying to delete load balancer from the cloud provider API
    CloudProviderDeleteLoadBalancer,
    // DoNotRespectCloudProviderBestPractices: represents an error, the user is trying to do something that is not recommended by the cloud provider
    DoNotRespectCloudProviderBestPractices,
    /// K8sCannotConnectToApi: represents an error when trying to contact K8s API.
    K8sCannotReachToApi,
    /// K8sPodDisruptionBudgetInInvalidState: represents an error where pod disruption budget is in an invalid state.
    K8sPodDisruptionBudgetInInvalidState,
    /// K8sPodDisruptionBudgetCqnnotBeRetrieved: represents an error where pod disruption budget cannot be retrieved.
    K8sPodsDisruptionBudgetCannotBeRetrieved,
    /// K8sCannotDeletePod: represents an error where we are not able to delete a pod.
    K8sCannotDeletePod,
    K8sCannotDeletePvc,
    /// K8sCannotGetCrashLoopingPods: represents an error where we are not able to get crash looping pods.
    K8sCannotGetCrashLoopingPods,
    /// K8sCannotDeleteCompletedJobs: represents an error where we are not able to delete completed jobs.
    K8sCannotDeleteCompletedJobs,
    /// K8sCannotGetPods: represents an error where we are not able to get pods.
    K8sCannotGetPods,
    /// K8sUpgradeDeployedVsRequestedVersionsInconsistency: represents an error where there is a K8s versions inconsistency between deployed and requested.
    K8sUpgradeDeployedVsRequestedVersionsInconsistency,
    /// K8sScaleReplicas: represents an error while trying to scale replicas.
    K8sScaleReplicas,
    /// K8sLoadBalancerConfigurationIssue: represents an error where loadbalancer has a configuration issue.
    K8sLoadBalancerConfigurationIssue,
    /// K8sServiceError: represents an error on a k8s service.
    K8sServiceError,
    /// K8sGetLogs: represents an error during a k8s logs command.
    K8sGetLogs,
    /// K8sGetEvents: represents an error during a k8s get events command.
    K8sGetEvents,
    /// K8sDescribe: represents an error during a k8s describe command.
    K8sDescribe,
    /// K8sHistory: represents an error during a k8s history command.
    K8sHistory,
    /// K8sCannotCreateNamespace: represents an error while trying to create a k8s namespace.
    K8sCannotCreateNamespace,
    /// K8sPodIsNotReady: represents an error where the given pod is not ready.
    K8sPodIsNotReady,
    /// K8sNodeIsNotReadyInTheGivenVersion: represents an error where the given node is not ready in the given version.
    K8sNodeIsNotReadyWithTheRequestedVersion,
    /// K8sNodeIsNotReady: represents an error where the given node is not ready.
    K8sNodeIsNotReady,
    /// K8sValidateRequiredCPUandBurstableError: represents an error validating required CPU and burstable.
    K8sValidateRequiredCPUandBurstableError,
    /// K8sErrorCopySecret: represents an error while copying secret from one namespace to another
    K8sErrorCopySecret,
    /// CannotFindRequiredBinary: represents an error where a required binary is not found on the system.
    CannotFindRequiredBinary,
    /// SubnetsCountShouldBeEven: represents an error where subnets count should be even to have as many public than private subnets.
    SubnetsCountShouldBeEven,
    /// CannotGetOrCreateIamRole: represents an error where we cannot get or create the given IAM role.
    CannotGetOrCreateIamRole,
    /// CannotCopyFilesFromDirectoryToDirectory: represents an error where we cannot copy files from one directory to another.
    CannotCopyFilesFromDirectoryToDirectory,
    /// CannotPauseClusterTasksAreRunning: represents an error where we cannot pause the cluster because some tasks are still running in the engine.
    CannotPauseClusterTasksAreRunning,
    /// TerraformUnknownError: terraform unknown error
    TerraformUnknownError,
    /// TerraformInvalidCredentials: terraform invalid cloud provider credentials
    TerraformInvalidCredentials,
    /// TerraformAccountBlockedByProvider: terraform cannot perform action because account has been blocked by cloud provider.
    TerraformAccountBlockedByProvider,
    /// TerraformMultipleInterruptsReceived: terraform received multiple interrupts
    TerraformMultipleInterruptsReceived,
    /// TerraformNotEnoughPermissions: terraform issue due to user not having enough permissions to perform action on the resource
    TerraformNotEnoughPermissions,
    /// TerraformWrongState: terraform issue due to wrong state of the resource
    TerraformWrongState,
    /// TerraformResourceDependencyViolation: terraform issue due to resource dependency violation
    TerraformResourceDependencyViolation,
    /// TerraformInstanceTypeDoesntExist: terraform issue due to instance type doesn't exist in the current region
    TerraformInstanceTypeDoesntExist,
    /// TerraformInstanceVolumeCannotBeReduced: terraform issue due to instance volume cannot be downsized
    TerraformInstanceVolumeCannotBeReduced,
    /// TerraformConfigFileNotFound: terraform config file cannot be found
    TerraformConfigFileNotFound,
    /// TerraformConfigFileInvalidContent: terraform config file has invalid content
    TerraformConfigFileInvalidContent,
    /// TerraformCannotDeleteLockFile: terraform cannot delete Lock file.
    TerraformCannotDeleteLockFile,
    /// TerraformInitError: terraform error while applying init command.
    TerraformInitError,
    /// TerraformValidateError: terraform error while applying validate command.
    TerraformValidateError,
    /// TerraformPlanError: terraform error while applying plan command.
    TerraformPlanError,
    /// TerraformApplyError: terraform error while applying apply command.
    TerraformApplyError,
    /// TerraformDestroyError: terraform error while applying apply destroy command.
    TerraformDestroyError,
    /// TerraformCannotRemoveEntryOut: represents an error where we cannot remove an entry out of Terraform.
    TerraformCannotRemoveEntryOut,
    /// TerraformErrorWhileExecutingPipeline: represents an error while executing Terraform pipeline.
    TerraformErrorWhileExecutingPipeline,
    /// TerraformErrorWhileExecutingDestroyPipeline: represents an error while executing Terraform destroying pipeline.
    TerraformErrorWhileExecutingDestroyPipeline,
    /// TerraformContextUnsupportedParameterValue: represents an error while trying to render terraform context because of unsupported parameter value.
    TerraformContextUnsupportedParameterValue,
    /// TerraformCloudProviderQuotasReached: represents an error due to cloud provider quotas exceeded.
    TerraformCloudProviderQuotasReached,
    /// TerraformCloudProviderActivationRequired: represents an error due to cloud provider requiring account to be validated first.
    TerraformCloudProviderActivationRequired,
    /// TerraformServiceNotActivatedOptInRequired: represents an error due to service not being
    /// activated on cloud account.
    TerraformServiceNotActivatedOptInRequired,
    /// TerraformWaitingTimeoutResource: represents an error due to resource being in flaky state in tf state.
    TerraformWaitingTimeoutResource,
    /// TerraformAlreadyExistingResource: represents an error due to resource already present in tf state while trying to create it.
    TerraformAlreadyExistingResource,
    /// TerraformInvalidCIDRBlock: represents an error due to an unusable CIDR block already used in the target VPC.
    TerraformInvalidCIDRBlock,
    /// TerraformStateLocked: represents an error due to Terraform state lock.
    TerraformStateLocked,
    /// HelmChartsSetupError: represents an error while trying to setup helm charts.
    HelmChartsSetupError,
    /// HelmChartsDeployError: represents an error while trying to deploy helm charts.
    HelmChartsDeployError,
    /// HelmChartsUpgradeError: represents an error while trying to upgrade helm charts.
    HelmChartsUpgradeError,
    /// HelmChartUninstallError: represents an error while trying to uninstall an helm chart.
    HelmChartUninstallError,
    /// HelmHistoryError: represents an error while trying to execute helm history on a helm chart.
    HelmHistoryError,
    /// HelmDeployTimeout: represent a failure to run the helm command in the given time frame
    HelmDeployTimeout,
    /// CannotGetAnyAvailableVPC: represents an error while trying to get any available VPC.
    CannotGetAnyAvailableVPC,
    /// UnsupportedVersion: represents an error where product doesn't support the given version.
    UnsupportedVersion,
    /// CannotGetSupportedVersions: represents an error while trying to get supported versions.
    CannotGetSupportedVersions,
    /// CannotGetCluster: represents an error where we cannot get cluster.
    CannotGetCluster,
    /// OnlyOneClusterExpected: represents an error where only one cluster was expected but several where found
    OnlyOneClusterExpected,
    /// ClientServiceFailedToStart: represent an error while trying to start a client's service.
    ClientServiceFailedToStart,
    /// ClientServiceFailedToDeployBeforeStart: represents an error while trying to deploy a client's service before start.
    ClientServiceFailedToDeployBeforeStart,
    /// DatabaseFailedToStartAfterSeveralRetries: represents an error while trying to start a database after several retries.
    DatabaseFailedToStartAfterSeveralRetries,
    /// RouterFailedToDeploy: represents an error while trying to deploy a router.
    RouterFailedToDeploy,
    /// CloudProviderInformationError: represents an error when checking cloud provider information provided.
    CloudProviderInformationError,
    /// CloudProviderClientInvalidCredentials: represents an error where client credentials for a cloud providers appear to be invalid.
    CloudProviderClientInvalidCredentials,
    /// CloudProviderApiMissingInfo: represents an error while expecting mandatory info
    CloudProviderApiMissingInfo,
    /// ServiceInvalidVersionNumberError: represents an error where the version number is not valid.
    VersionNumberParsingError,
    /// NotImplementedError: represents an error where feature / code has not been implemented yet.
    NotImplementedError,
    /// TaskCancellationRequested: represents an error where current task cancellation has been requested.
    TaskCancellationRequested,
    /// BuildError: represents an error when trying to build an application.
    BuilderError,
    /// BuilderDockerCannotFindAnyDockerfile: represents an error when trying to get a Dockerfile.
    BuilderDockerCannotFindAnyDockerfile,
    /// BuilderDockerCannotReadDockerfile: represents an error while trying to read Dockerfile.
    BuilderDockerCannotReadDockerfile,
    /// BuilderDockerCannotExtractEnvVarsFromDockerfile: represents an error while trying to extract ENV vars from Dockerfile.
    BuilderDockerCannotExtractEnvVarsFromDockerfile,
    /// BuilderDockerCannotBuildContainerImage: represents an error while trying to build Docker container image.
    BuilderDockerCannotBuildContainerImage,
    /// BuilderDockerCannotListImages: represents an error while trying to list docker images.
    BuilderDockerCannotListImages,
    /// BuilderBuildpackInvalidLanguageFormat: represents an error where buildback requested language has wrong format.
    BuilderBuildpackInvalidLanguageFormat,
    /// BuilderBuildpackCannotBuildContainerImage: represents an error while trying to build container image with Buildpack.
    BuilderBuildpackCannotBuildContainerImage,
    /// BuilderGetBuildError: represents an error when builder is trying to get parent build.
    BuilderGetBuildError,
    /// BuilderCloningRepositoryError: represents an error when builder is trying to clone a git repository.
    BuilderCloningRepositoryError,
    /// DockerError: represents an error when trying to use docker cli.
    DockerError,
    /// DockerPushImageError: represents an error when trying to push a docker image.
    DockerPushImageError,
    /// DockerPullImageError: represents an error when trying to pull a docker image.
    DockerPullImageError,
    /// ContainerRegistryCannotCreateRepository: represents an error when trying to create a repository.
    ContainerRegistryCannotCreateRepository,
    /// ContainerRegistryCannotSetRepositoryLifecycle: represents an error when trying to set repository lifecycle policy.
    ContainerRegistryCannotSetRepositoryLifecycle,
    /// ContainerRegistryCannotGetCredentials: represents an error when trying to get container registry credentials.
    ContainerRegistryCannotGetCredentials,
    /// ContainerRegistryCannotDeleteImage: represents an error while trying to delete an image.
    ContainerRegistryCannotDeleteImage,
    /// ContainerRegistryImageDoesntExist: represents an error, image doesn't exist in the registry.
    ContainerRegistryImageDoesntExist,
    /// ContainerRegistryImageUnreachableAfterPush: represents an error when image has been pushed but is unreachable.
    ContainerRegistryImageUnreachableAfterPush,
    /// ContainerRegistryRepositoryDoesntExistInRegistry: represents an error, repository doesn't exist in registry.
    ContainerRegistryRepositoryDoesntExistInRegistry,
    /// ContainerRegistryRegistryDoesntExist: represents an error, registry doesn't exist.
    ContainerRegistryRegistryDoesntExist,
    /// ContainerRegistryCannotDeleteRepository: represents an error while trying to delete a repository.
    ContainerRegistryCannotDeleteRepository,
    /// ContainerRegistryInvalidInformation: represents an error on container registry information provided.
    ContainerRegistryInvalidInformation,
    /// ContainerRegistryInvalidCredentials: represents an error on container registry, credentials are not valid.
    ContainerRegistryInvalidCredentials,
    /// ContainerRegistryCannotLinkRegistryToCluster: represents an error on container registry where it cannot be linked to cluster
    ContainerRegistryCannotLinkRegistryToCluster,
    /// ContainerRegistryCannotCreateRegistry: represents an error on container registry where it cannot create a registry.
    ContainerRegistryCannotCreateRegistry,
    /// ContainerRegistryCannotDeleteRegistry: represents an error on container registry where it cannot delete a registry.
    ContainerRegistryCannotDeleteRegistry,
    /// ContainerRegistryCannotSetTags: represents an error on container registry where it cannot cannot set tags.
    ContainerRegistryCannotSetRepositoryTags,
    /// ContainerRegistryCannotSetTags: represents an unknown error on container registry.
    ContainerRegistryUnknownError,
    /// KubeconfigFileDoNotPermitToConnectToK8sCluster: represent a kubeconfig mismatch, not permitting to connect to k8s cluster
    KubeconfigFileDoNotPermitToConnectToK8sCluster,
    /// KubeconfigSecurityCheckError: represent an error because of a security concern/doubt on the kubeconfig file
    KubeconfigSecurityCheckError,
    /// DeleteLocalKubeconfigFileError: represent an error when trying to delete Kubeconfig
    DeleteLocalKubeconfigFileError,
    /// VaultConnectionError: represents an error while trying to connect ot Vault service
    VaultConnectionError,
    /// VaultSecretCouldNotBeRetrieved: represents an error to get the desired secret
    VaultSecretCouldNotBeRetrieved,
    /// VaultSecretCouldNotBeCreatedOrUpdated: represent a vault secret creation or update error
    VaultSecretCouldNotBeCreatedOrUpdated,
    /// VaultSecretCouldNotBeDeleted, represent a vault secret deletion error
    VaultSecretCouldNotBeDeleted,
    /// JsonDeserializationError: represent a deserialization issue
    JsonDeserializationError,
    /// ClusterSecretsManipulationError: represent an error while trying to manipulate ClusterSecrets
    ClusterSecretsManipulationError,
    /// DnsProviderInformationError: represent an error on DNS provider information provided.
    DnsProviderInformationError,
    /// DnsProviderInvalidCredentials: represent an error on invalid DNS provider credentials.
    DnsProviderInvalidCredentials,
    /// DnsProviderInvalidApiUrl: represent an error on invalid DNS provider api url.
    DnsProviderInvalidApiUrl,
    /// ObjectStorageCannotCreateBucket: represents an error while trying to create a new object storage bucket.
    ObjectStorageCannotCreateBucket,
    /// ObjectStorageCannotPutFileIntoBucket: represents an error while trying to put a file into an object storage bucket.
    ObjectStorageCannotPutFileIntoBucket,
    /// ObjectStorageCannotDeleteFileIntoBucket: represents an error while trying to delete a file into an object storage bucket.
    ObjectStorageCannotDeleteFileIntoBucket,
    /// ObjectStorageCannotDeleteBucket: represents an error while trying to delete a bucket.
    ObjectStorageCannotDeleteBucket,
    /// ObjectStorageCannotActivateBucketVersioning: represents an error while trying to activate bucket versioning for bucket.
    ObjectStorageCannotActivateBucketVersioning,
    /// ObjectStorageQuotaExceeded: represents an error, quotas has been exceeded.
    ObjectStorageQuotaExceeded,
    /// ObjectStorageInvalidBucketName: represents an error, bucket name is not valid.
    ObjectStorageInvalidBucketName,
    /// ObjectStorageCannotEmptyBucket: represents an error while trying to empty an object storage bucket.
    ObjectStorageCannotEmptyBucket,
    /// ObjectStorageCannotTagBucket: represents an error while trying to tag an object storage bucket.
    ObjectStorageCannotTagBucket,
    /// ObjectStorageCannotGetObjectFile: represents an error while trying to get a file from object storage bucket.
    ObjectStorageCannotGetObjectFile,
    /// JobFailure: represents an error while indicating that the job failed to terminate properly
    JobFailure,
}

impl Tag {
    pub fn is_cancel(&self) -> bool {
        matches!(self, Tag::TaskCancellationRequested)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// EngineError: represents an engine error. Engine will always returns such errors carrying context infos easing monitoring and debugging.
pub struct EngineError {
    /// tag: error unique identifier
    tag: Tag,
    /// event_details: holds context details in which error was triggered such as organization ID, cluster ID, etc.
    event_details: EventDetails,
    /// user_log_message: message targeted toward Qovery users, might avoid any useless info for users such as Qovery specific identifiers and so on.
    user_log_message: String,
    /// underlying_error: raw error message such as command input / output.
    underlying_error: Option<CommandError>,
    /// link: link to error documentation (qovery blog, forum, etc.)
    link: Option<Url>,
    /// hint_message: an hint message aiming to give an hint to the user. For example: "Happens when application port has been changed but application hasn't been restarted.".
    hint_message: Option<String>,
}

impl EngineError {
    /// Returns error's unique identifier.
    pub fn tag(&self) -> &Tag {
        &self.tag
    }

    /// Returns error's event details.
    pub fn event_details(&self) -> &EventDetails {
        &self.event_details
    }

    /// Returns user log message.
    pub fn user_log_message(&self) -> &str {
        &self.user_log_message
    }

    /// Returns proper error message.
    pub fn message(&self, message_verbosity: ErrorMessageVerbosity) -> String {
        match &self.underlying_error {
            Some(msg) => msg.message(message_verbosity),
            None => self.user_log_message.to_string(),
        }
    }

    /// Returns Engine's underlying error.
    pub fn underlying_error(&self) -> Option<CommandError> {
        self.underlying_error.clone()
    }

    /// Returns error's link.
    pub fn link(&self) -> &Option<Url> {
        &self.link
    }

    /// Returns error's hint message.
    pub fn hint_message(&self) -> &Option<String> {
        &self.hint_message
    }

    /// Creates new EngineError.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `tag`: Error unique identifier.
    /// * `user_log_message`: Error log message targeting Qovery user, avoiding any extending pointless details.
    /// * `underlying_error`: raw error message such as command input / output.
    /// * `link`: Link documenting the given error.
    /// * `hint_message`: hint message aiming to give an hint to the user. For example: "Happens when application port has been changed but application hasn't been restarted.".
    fn new(
        mut event_details: EventDetails,
        tag: Tag,
        user_log_message: String,
        underlying_error: Option<CommandError>,
        link: Option<Url>,
        hint_message: Option<String>,
    ) -> Self {
        if tag.is_cancel() {
            event_details.mut_to_cancel_stage()
        } else {
            event_details.mut_to_error_stage()
        }

        EngineError {
            event_details,
            tag,
            user_log_message,
            underlying_error,
            link,
            hint_message,
        }
    }
    /// Clone an existing engine error to specify a stage
    ///
    /// Arguments:
    ///
    /// * `stage`: stage that replaces the current stage of the engine error
    pub fn clone_engine_error_with_stage(&self, stage: Stage) -> Self {
        EngineError {
            event_details: EventDetails::new(
                self.event_details.provider_kind(),
                self.event_details.organisation_id().clone(),
                self.event_details.cluster_id().clone(),
                self.event_details.execution_id().to_string(),
                stage,
                self.event_details.transmitter(),
            ),
            tag: self.tag.clone(),
            user_log_message: self.user_log_message.clone(),
            underlying_error: self.underlying_error.as_ref().cloned(),
            link: self.link.as_ref().cloned(),
            hint_message: self.hint_message.as_ref().cloned(),
        }
    }

    /// Creates new unknown error.
    ///
    /// Note: do not use unless really needed, every error should have a clear type.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `user_log_message`: Error log message targeting Qovery user, avoiding any extending pointless details.
    /// * `underlying_error`: raw error message such as command input / output.
    /// * `link`: Link documenting the given error.
    /// * `hint_message`: hint message aiming to give an hint to the user. For example: "Happens when application port has been changed but application hasn't been restarted.".
    pub fn new_unknown(
        event_details: EventDetails,
        user_log_message: String,
        underlying_error: Option<CommandError>,
        link: Option<Url>,
        hint_message: Option<String>,
    ) -> EngineError {
        EngineError::new(
            event_details,
            Tag::Unknown,
            user_log_message,
            underlying_error,
            link,
            hint_message,
        )
    }

    /// Creates new from an engine error. Only change the use log message and hint
    ///
    /// Arguments:
    ///
    /// * `engine_error`: Source engine error
    /// * `user_log_message`: Error log message targeting Qovery user, avoiding any extending pointless details.
    /// * `underlying_error`: raw error message such as command input / output.
    pub fn new_engine_error(
        mut engine_error: EngineError,
        user_log_message: String,
        hint_message: Option<String>,
    ) -> EngineError {
        engine_error.user_log_message = user_log_message;
        engine_error.hint_message = hint_message;
        engine_error.underlying_error = None;
        engine_error.link = None;

        engine_error
    }

    /// Creates new error for Engine API input cannot be deserialized.
    ///
    ///
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw serde message.
    pub fn new_invalid_engine_api_input_cannot_be_deserialized(
        event_details: EventDetails,
        raw_error: serde_json::Error,
    ) -> EngineError {
        EngineError::new(
            event_details,
            Tag::InvalidEngineApiInputCannotBeDeserialized,
            "Input is invalid and cannot be deserialized.".to_string(),
            Some(raw_error.into()),
            None,
            Some("This is a Qovery issue, please contact our support team".to_string()),
        )
    }

    /// Creates new error for Engine API payload that are not valid.
    ///
    ///
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `message`: Raw error message.
    pub fn new_invalid_engine_payload(event_details: EventDetails, message: &str) -> EngineError {
        EngineError::new(
            event_details,
            Tag::InvalidEnginePayload,
            format!("Input is invalid and cannot be executed by the engine: {}", message),
            None,
            None,
            Some("This is a Qovery issue, please contact our support team".to_string()),
        )
    }

    pub fn new_job_error(event_details: EventDetails, message: String) -> EngineError {
        EngineError::new(event_details, Tag::JobFailure, message, None, None, None)
    }

    /// Creates new error for missing required env variable.
    ///
    ///
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `variable_name`: Variable name which is not set.
    pub fn new_missing_required_env_variable(event_details: EventDetails, variable_name: String) -> EngineError {
        let message = format!("`{}` environment variable wasn't found.", variable_name);
        EngineError::new(event_details, Tag::MissingRequiredEnvVariable, message, None, None, None)
    }

    /// Creates new error for cluster has no worker nodes.
    ///
    ///
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_cluster_has_no_worker_nodes(
        event_details: EventDetails,
        raw_error: Option<CommandError>,
    ) -> EngineError {
        let message = "No worker nodes present, can't proceed with operation.";
        EngineError::new(
            event_details,
            Tag::ClusterHasNoWorkerNodes,
            message.to_string(),
            raw_error,
            None,
            Some(
                "This can happen if there where a manual operations on the workers or the infrastructure is paused."
                    .to_string(),
            ),
        )
    }

    /// Creates new error when copying secrets from a namespace to another
    ///
    ///
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_copy_secrets_to_another_namespace_error(
        event_details: EventDetails,
        raw_error: KubeError,
        from_namespace: &str,
        to_namespace: &str,
    ) -> EngineError {
        let message = format!(
            "error while copying secret from namespace {} to {}",
            from_namespace, to_namespace
        );
        let cmd_err = CommandError::new(message.clone(), Some(format!("{:?}", raw_error)), None);
        EngineError::new(event_details, Tag::K8sErrorCopySecret, message, Some(cmd_err), None, None)
    }

    /// Creates new error for cluster worker node is not found.
    ///
    ///
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_cluster_worker_node_not_found(
        event_details: EventDetails,
        raw_error: Option<CommandError>,
    ) -> EngineError {
        let message = "Worker node not found, can't proceed with operation.";
        EngineError::new(
            event_details,
            Tag::ClusterWorkerNodeNotFound,
            message.to_string(),
            raw_error,
            None,
            Some(
                "This can happen if there where a manual operations on the workers or the infrastructure is paused."
                    .to_string(),
            ),
        )
    }

    /// Missing API info from the Cloud provider itself
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_missing_api_info_from_cloud_provider_error(
        event_details: EventDetails,
        raw_error: Option<CommandError>,
    ) -> EngineError {
        let message = "Error, missing required information from the Cloud Provider API";
        EngineError::new(
            event_details,
            Tag::CloudProviderApiMissingInfo,
            message.to_string(),
            raw_error,
            None,
            Some(
                "This can happen if the cloud provider is encountering issues. You should try again later".to_string(),
            ),
        )
    }

    /// Creates new error for not allowed instance type.
    ///
    /// Qovery doesn't allow the requested instance type.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `requested_instance_type`: Raw requested instance type string.
    pub fn new_not_allowed_instance_type(event_details: EventDetails, requested_instance_type: &str) -> EngineError {
        let message = format!(
            "`{}` instance type is not allowed for this kind of cluster",
            requested_instance_type
        );
        EngineError::new(
            event_details,
            Tag::NotAllowedInstanceType,
            message,
            None,
            None, // TODO(documentation): Create a page entry to details this error
            Some("Selected instance type is not allowed, please check Qovery's documentation.".to_string()),
        )
    }

    /// Creates new error for unsupported instance type.
    ///
    /// Cloud provider doesn't support the requested instance type.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `requested_instance_type`: Raw requested instance type string.
    /// * `error_message`: Raw error message.
    pub fn new_unsupported_instance_type(
        event_details: EventDetails,
        requested_instance_type: &str,
        error_message: CommandError,
    ) -> EngineError {
        let message = format!("`{}` instance type is not supported", requested_instance_type);
        EngineError::new(
            event_details,
            Tag::UnsupportedInstanceType,
            message,
            Some(error_message),
            None, // TODO(documentation): Create a page entry to details this error
            Some("Selected instance type is not supported, please check provider's documentation.".to_string()),
        )
    }

    /// Creates new error for unsupported cluster kind.
    ///
    /// Qovery doesn't support this kind of clusters.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `requested_kind`: Raw requested instance type string.
    /// * `error_message`: Raw error message.
    pub fn new_unsupported_cluster_kind(
        event_details: EventDetails,
        new_unsupported_cluster_kind: &str,
        error_message: CommandError,
    ) -> EngineError {
        let message = format!("`{}` cluster kind is not supported", new_unsupported_cluster_kind);
        EngineError::new(
            event_details,
            Tag::UnsupportedClusterKind,
            message,
            Some(error_message),
            None, // TODO(documentation): Create a page entry to details this error
            Some("Selected cluster kind is not supported, please check Qovery's documentation.".to_string()),
        )
    }

    /// Creates new error for unsupported region.
    ///
    /// Cloud provider doesn't support the requested region.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `requested_region`: Raw requested region string.
    /// * `error_message`: Raw error message.
    pub fn new_unsupported_region(
        event_details: EventDetails,
        requested_region: String,
        error_message: CommandError,
    ) -> EngineError {
        let message = format!("`{}` region is not supported", requested_region);
        EngineError::new(
            event_details,
            Tag::UnsupportedRegion,
            message,
            Some(error_message),
            None, // TODO(documentation): Create a page entry to details this error
            Some("Selected region is not supported, please check provider's documentation.".to_string()),
        )
    }

    /// Creates new error for unsupported zone for region.
    ///
    /// Cloud provider doesn't support the requested zone in region.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `region`: Raw requested region string.
    /// * `requested_zone`: Raw requested zone string.
    /// * `error_message`: Raw error message.
    pub fn new_unsupported_zone(
        event_details: EventDetails,
        region: String,
        requested_zone: String,
        error_message: CommandError,
    ) -> EngineError {
        let message = format!("Zone `{}` is not supported in region `{}`.", requested_zone, region);
        EngineError::new(
            event_details,
            Tag::UnsupportedZone,
            message,
            Some(error_message),
            None, // TODO(documentation): Create a page entry to details this error
            Some("Selected zone is not supported in the region, please check provider's documentation.".to_string()),
        )
    }

    /// Creates new error: cannot get workspace directory.
    ///
    /// Error occurred while trying to get workspace directory.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `error_message`: Raw error message.
    pub fn new_cannot_get_workspace_directory(event_details: EventDetails, error_message: CommandError) -> EngineError {
        let message = "Error while trying to get workspace directory";
        EngineError::new(
            event_details,
            Tag::CannotGetWorkspaceDirectory,
            message.to_string(),
            Some(error_message),
            None,
            None,
        )
    }

    /// Creates new error for cluster configuration file couldn't be retrieved.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `error_message`: Raw error message.
    pub fn new_cannot_retrieve_cluster_config_file(
        event_details: EventDetails,
        error_message: CommandError,
    ) -> EngineError {
        let message = "Cannot retrieve Kubernetes kubeconfig";
        EngineError::new(
            event_details,
            Tag::CannotRetrieveClusterConfigFile,
            message.to_string(),
            Some(error_message),
            None,
            None,
        )
    }

    /// Creates new error for file we can't create.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `error_message`: Raw error message.
    pub fn new_cannot_create_file(event_details: EventDetails, error_message: CommandError) -> EngineError {
        let message = "Cannot create file";
        EngineError::new(
            event_details,
            Tag::CannotCreateFile,
            message.to_string(),
            Some(error_message),
            None,
            None,
        )
    }

    /// Creates new error for Kubernetes cannot get nodes.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `error_message`: Raw error message.
    pub fn new_cannot_get_cluster_nodes(event_details: EventDetails, error_message: CommandError) -> EngineError {
        let message = "Cannot get Kubernetes nodes";
        EngineError::new(
            event_details,
            Tag::CannotRetrieveClusterConfigFile,
            message.to_string(),
            Some(error_message),
            None,
            None,
        )
    }

    /// Creates new error for cannot deploy because the desired number of nodes is greater than the maximum allowed.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `actual_nodes`: The actual number of nodes running.
    /// * `max_nodes`: The maximum number of nodes allowed.
    pub fn new_cannot_deploy_max_nodes_exceeded(
        event_details: EventDetails,
        actual_nodes: i32,
        max_nodes: i32,
    ) -> EngineError {
        let message = format!(
            "The actual number of nodes {} can't be greater than the maximum value {}",
            actual_nodes, max_nodes
        );

        EngineError::new(
            event_details,
            Tag::NotEnoughNodesAvailableToDeployEnvironment,
            message,
            None,
            None,
            Some("Consider to upgrade your nodes configuration.".to_string()),
        )
    }

    /// error explaining to the user the requested maximum of nodes is below the current usage
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `current_node_number`: The actual number of nodes running.
    /// * `max_nodes`: The maximum number of nodes allowed.
    pub fn new_number_of_requested_max_nodes_is_below_than_current_usage_error(
        event_details: EventDetails,
        current_node_number: i32,
        max_nodes: i32,
    ) -> EngineError {
        EngineError::new(
            event_details,
            Tag::NumberOfRequestedMaxNodesIsBelowThanCurrentUsage,
            format!(
                "The actual number of nodes {} is above than the maximum number ({}) requested.",
                current_node_number, max_nodes
            ),
            None,
            None,
            Some("Reduce your resources usage or set it to a higher value".to_string()),
        )
    }

    /// Creates new error for cannot deploy because there are not enough available resources on the cluster.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `requested_ram_in_mib`: How much RAM in mib is requested.
    /// * `free_ram_in_mib`: How much RAM in mib is free.
    /// * `requested_cpu`: How much CPU is requested.
    /// * `free_cpu`: How much CPU is free.
    pub fn new_cannot_deploy_not_enough_resources_available(
        event_details: EventDetails,
        requested_ram_in_mib: u32,
        free_ram_in_mib: u32,
        requested_cpu: f32,
        free_cpu: f32,
    ) -> EngineError {
        let mut message = vec!["There is not enough resources on the cluster:".to_string()];

        if requested_cpu > free_cpu {
            message.push(format!("{} CPU requested and only {} CPU available", free_cpu, requested_cpu));
        }

        if requested_ram_in_mib > free_ram_in_mib {
            message.push(format!(
                "{}mib RAM requested and only {}mib RAM  available",
                requested_ram_in_mib, free_ram_in_mib
            ));
        }

        let message = message.join("\n");

        EngineError::new(
            event_details,
            Tag::NotEnoughResourcesToDeployEnvironment,
            message,
            None,
            None,
            Some("Consider to add one more node or upgrade your nodes configuration. If not possible, pause or delete unused environments.".to_string()),
        )
    }

    /// Creates new error for cannot deploy because there are not enough free pods available on the cluster.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `requested_pods`: How many pods are requested.
    /// * `free_pods`: How many pods qre free.
    pub fn new_cannot_deploy_not_enough_free_pods_available(
        event_details: EventDetails,
        requested_pods: u32,
        free_pods: u32,
    ) -> EngineError {
        let message = format!(
            "There is not enough free Pods (free {} VS {} required) on the cluster.",
            free_pods, requested_pods,
        );

        EngineError::new(
            event_details,
            Tag::NotEnoughResourcesToDeployEnvironment,
            message,
            None,
            None,
            Some("Consider to add one more node or upgrade your nodes configuration. If not possible, pause or delete unused environments.".to_string()),
        )
    }

    /// Creates new error for cannot uninstall an helm chart.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `helm_chart_name`: Helm chart name.
    /// * `errored_object_kind`: Errored kubernetes object name.
    /// * `raw_error`: Raw error message.
    pub fn new_cannot_uninstall_helm_chart(
        event_details: EventDetails,
        helm_chart_name: String,
        errored_object_kind: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!(
            "Wasn't able to delete all objects type {}, it's a blocker to then delete cert-manager namespace. {}",
            helm_chart_name, errored_object_kind,
        );

        EngineError::new(
            event_details,
            Tag::CannotUninstallHelmChart,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error for cannot exec K8s version.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `error_message`: Raw error message.
    pub fn new_cannot_execute_k8s_exec_version(
        event_details: EventDetails,
        error_message: CommandError,
    ) -> EngineError {
        let message = "Unable to execute Kubernetes exec version: `{}`";

        EngineError::new(
            event_details,
            Tag::CannotExecuteK8sVersion,
            message.to_string(),
            Some(error_message),
            None,
            None,
        )
    }

    /// Creates new error for cannot determine kubernetes master version.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `kubernetes_raw_version`: Kubernetes master raw version string we tried to get version from.
    pub fn new_cannot_determine_k8s_master_version(
        event_details: EventDetails,
        kubernetes_raw_version: String,
    ) -> EngineError {
        let message = format!("Unable to determine Kubernetes master version: `{}`", kubernetes_raw_version,);

        EngineError::new(event_details, Tag::CannotDetermineK8sMasterVersion, message, None, None, None)
    }

    /// Creates new error for cannot determine kubernetes master version.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `kubernetetest_terraform_error_aws_permissions_issues_upgrade_requested_raw_version`: Kubernetes requested upgrade raw version string.
    /// * `error_message`: Raw error message.
    pub fn new_cannot_determine_k8s_requested_upgrade_version(
        event_details: EventDetails,
        kubernetes_upgrade_requested_raw_version: String,
        error_message: Option<CommandError>,
    ) -> EngineError {
        let message = format!(
            "Unable to determine Kubernetes upgrade requested version: `{}`. Upgrade is not possible.",
            kubernetes_upgrade_requested_raw_version,
        );

        EngineError::new(
            event_details,
            Tag::CannotDetermineK8sRequestedUpgradeVersion,
            message,
            error_message,
            None,
            None,
        )
    }

    /// Creates new error for cannot determine kubernetes kubelet worker version.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `kubelet_worker_raw_version`: Kubelet raw version string we tried to get version from.
    pub fn new_cannot_determine_k8s_kubelet_worker_version(
        event_details: EventDetails,
        kubelet_worker_raw_version: String,
    ) -> EngineError {
        let message = format!("Unable to determine Kubelet worker version: `{}`", kubelet_worker_raw_version,);

        EngineError::new(
            event_details,
            Tag::CannotDetermineK8sKubeletWorkerVersion,
            message,
            None,
            None,
            None,
        )
    }

    /// Creates new error for cannot determine kubernetes kube proxy version.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `kube_proxy_raw_version`: Kube proxy raw version string we tried to get version from.
    pub fn new_cannot_determine_k8s_kube_proxy_version(
        event_details: EventDetails,
        kube_proxy_raw_version: String,
    ) -> EngineError {
        let message = format!("Unable to determine Kube proxy version: `{}`", kube_proxy_raw_version,);

        EngineError::new(
            event_details,
            Tag::CannotDetermineK8sKubeProxyVersion,
            message,
            None,
            None,
            None,
        )
    }

    pub fn new_cannot_pause_managed_database(event_details: EventDetails, command_error: CommandError) -> EngineError {
        let message = format!("Unable to pause managed database: {}", command_error.message_safe);

        EngineError::new(
            event_details,
            Tag::CannotPauseManagedDatabase,
            message,
            Some(command_error),
            None,
            None,
        )
    }

    pub fn new_cannot_connect_to_k8s_cluster(event_details: EventDetails, kube_error: kube::Error) -> EngineError {
        let message = format!("Unable to connect to target k8s cluster: `{}`", kube_error);

        EngineError::new(event_details, Tag::CannotConnectK8sCluster, message, None, None, None)
    }

    /// Creates new error delete local kubeconfig file error
    ///
    /// This is useful for EC2 when a kubeconfig stored in S3 do not match the current kubernetes
    /// certificates and/or endpoint
    ///
    /// Arguments:
    /// * `event_details`: Error linked event details.
    /// * `kubeconfig_path`: Kubeconfig path
    pub fn new_delete_local_kubeconfig_file_error(
        event_details: EventDetails,
        kubeconfig_path: &str,
        err: Error,
    ) -> EngineError {
        EngineError::new(
            event_details,
            Tag::DeleteLocalKubeconfigFileError,
            format!("Wasn't able to delete local kubeconfig file: `{}`", kubeconfig_path),
            Some(CommandError::from(err)),
            None,
            None,
        )
    }

    /// Creates new error to catch wrong kubeconfig file content
    ///
    /// This is useful for EC2 when a kubeconfig stored in S3 do not match the current kubernetes
    /// certificates and/or endpoint
    ///
    /// Arguments:
    /// * `event_details`: Error linked event details.
    pub fn new_kubeconfig_file_do_not_match_the_current_cluster(event_details: EventDetails) -> EngineError {
        EngineError::new(
            event_details,
            Tag::KubeconfigFileDoNotPermitToConnectToK8sCluster,
            "The kubeconfig stored in the S3 bucket is not valid and do not permit to connect to your current cluster"
                .to_string(),
            None,
            None,
            None,
        )
    }

    /// Creates new error to catch kubeconfig security issues
    ///
    /// Ensure kubeconfig is not corrupted
    ///
    /// Arguments:
    /// * `event_details`: Error linked event details.
    /// * `current_size`: Current size of the kubeconfig file
    /// * `max_size`: Maximum authorized size of the kubeconfig file
    pub fn new_kubeconfig_size_security_check_error(
        event_details: EventDetails,
        current_size: u64,
        max_size: u64,
    ) -> EngineError {
        let message = format!(
            "The kubeconfig stored in the S3 bucket did not pass our security check. Kubeconfig file size ({}k), exceed authorized kubeconfig size (< {}k)",
            current_size,
            max_size
        );

        EngineError::new(event_details, Tag::KubeconfigSecurityCheckError, message, None, None, None)
    }

    /// Creates new error for cannot get api custom metrics.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_k8s_error`: Raw error message.
    pub fn new_cannot_get_k8s_api_custom_metrics(
        event_details: EventDetails,
        raw_k8s_error: CommandError,
    ) -> EngineError {
        let message = "Error while looking at the API metric value";

        EngineError::new(
            event_details,
            Tag::CannotExecuteK8sApiCustomMetrics,
            message.to_string(),
            Some(raw_k8s_error),
            None,
            None,
        )
    }

    /// Creates new error for kubernetes API cannot be reached.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    pub fn new_k8s_cannot_reach_api(event_details: EventDetails) -> EngineError {
        EngineError::new(
            event_details,
            Tag::K8sCannotReachToApi,
            "Kubernetes API cannot be reached.".to_string(),
            None,
            None,
            Some("Did you manually performed changes AWS side?".to_string()),
        )
    }

    /// Creates new error for kubernetes pod disruption budget being in an invalid state.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `pod_name`: Pod name having PDB in an invalid state.
    pub fn new_k8s_pod_disruption_budget_invalid_state(event_details: EventDetails, pod_name: String) -> EngineError {
        let message = format!("Unable to upgrade Kubernetes, pdb for app `{}` in invalid state.", pod_name,);

        EngineError::new(
            event_details,
            Tag::K8sPodDisruptionBudgetInInvalidState,
            message,
            None,
            None,
            None,
        )
    }

    /// Creates new error for kubernetes not being able to retrieve pods disruption budget.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_k8s_error`: Raw error message.
    pub fn new_k8s_cannot_retrieve_pods_disruption_budget(
        event_details: EventDetails,
        raw_k8s_error: CommandError,
    ) -> EngineError {
        let message = "Unable to upgrade Kubernetes, can't get pods disruptions budgets.";

        EngineError::new(
            event_details,
            Tag::K8sPodsDisruptionBudgetCannotBeRetrieved,
            message.to_string(),
            Some(raw_k8s_error),
            None,
            None,
        )
    }

    /// Creates new error for kubernetes not being able to delete a pod.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `pod_name`: Pod's name.
    /// * `raw_k8s_error`: Raw error message.
    pub fn new_k8s_cannot_delete_pod(
        event_details: EventDetails,
        pod_name: String,
        raw_k8s_error: CommandError,
    ) -> EngineError {
        let message = format!("Unable to delete Kubernetes pod `{}`.", pod_name);

        EngineError::new(event_details, Tag::K8sCannotDeletePod, message, Some(raw_k8s_error), None, None)
    }

    /// Creates new error for kubernetes not being able to delete a pvc.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `pvc_name`: Pvc's name.
    /// * `raw_k8s_error`: Raw error message.
    pub fn new_k8s_cannot_delete_pvcs(
        event_details: EventDetails,
        pvc_name: String,
        raw_k8s_error: CommandError,
    ) -> EngineError {
        let message = format!("Unable to delete Kubernetes pvc `{}`.", pvc_name);
        EngineError::new(event_details, Tag::K8sCannotDeletePvc, message, Some(raw_k8s_error), None, None)
    }

    /// Creates new error for kubernetes not being able to get crash looping pods.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_k8s_error`: Raw error message.
    pub fn new_k8s_cannot_get_crash_looping_pods(
        event_details: EventDetails,
        raw_k8s_error: CommandError,
    ) -> EngineError {
        let message = "Unable to get crash looping Kubernetes pods.";

        EngineError::new(
            event_details,
            Tag::K8sCannotGetCrashLoopingPods,
            message.to_string(),
            Some(raw_k8s_error),
            None,
            None,
        )
    }

    /// Creates new error for kubernetes not being able to delete completed jobs.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_k8s_error`: Raw error message.
    pub fn new_k8s_cannot_delete_completed_jobs(
        event_details: EventDetails,
        raw_k8s_error: CommandError,
    ) -> EngineError {
        let message = "Unable to delete completed Kubernetes jobs.";

        EngineError::new(
            event_details,
            Tag::K8sCannotDeleteCompletedJobs,
            message.to_string(),
            Some(raw_k8s_error),
            None,
            None,
        )
    }

    /// Creates new error for kubernetes not being able to get pods.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_k8s_error`: Raw error message.
    pub fn new_k8s_cannot_get_pods(event_details: EventDetails, raw_k8s_error: CommandError) -> EngineError {
        let message = "Unable to get Kubernetes pods.";

        EngineError::new(
            event_details,
            Tag::K8sCannotGetPods,
            message.to_string(),
            Some(raw_k8s_error),
            None,
            None,
        )
    }

    /// Creates new error for kubernetes upgrade version inconsistency.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `deployed_version`: Deployed k8s version.
    /// * `requested_version`: Requested k8s version.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_version_upgrade_deployed_vs_requested_versions_inconsistency(
        event_details: EventDetails,
        deployed_version: VersionsNumber,
        requested_version: VersionsNumber,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!(
            "Unable to upgrade Kubernetes due to version inconsistency. Deployed version: {}, requested version: {}.",
            deployed_version, requested_version
        );

        EngineError::new(
            event_details,
            Tag::K8sUpgradeDeployedVsRequestedVersionsInconsistency,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error for kubernetes scale replicas.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `selector`: K8s selector.
    /// * `namespace`: K8s namespace.
    /// * `requested_replicas`: Number of requested replicas.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_scale_replicas(
        event_details: EventDetails,
        selector: String,
        namespace: String,
        requested_replicas: u32,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!(
            "Unable to scale Kubernetes `{}` replicas to `{}` in namespace `{}`.",
            selector, requested_replicas, namespace,
        );

        EngineError::new(event_details, Tag::K8sScaleReplicas, message, Some(raw_error), None, None)
    }

    /// Creates new error for kubernetes load balancer configuration issue.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_loadbalancer_configuration_issue(
        event_details: EventDetails,
        raw_error: CommandError,
    ) -> EngineError {
        let message = "Error, there is an issue with loadbalancer configuration.";

        EngineError::new(
            event_details,
            Tag::K8sLoadBalancerConfigurationIssue,
            message.to_string(),
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error for kubernetes service issue.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_service_issue(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message = "Error, there is an issue with service.";

        EngineError::new(
            event_details,
            Tag::K8sServiceError,
            message.to_string(),
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error for kubernetes get logs.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `selector`: Selector to get logs for.
    /// * `namespace`: Resource's namespace to get logs for.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_get_logs_error(
        event_details: EventDetails,
        selector: String,
        namespace: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!(
            "Error, unable to retrieve logs for pod with selector `{}` in namespace `{}`.",
            selector, namespace
        );

        EngineError::new(event_details, Tag::K8sGetLogs, message, Some(raw_error), None, None)
    }

    /// Creates new error for kubernetes get events.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `namespace`: Resource's namespace to get json events for.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_get_json_events(
        event_details: EventDetails,
        namespace: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Error, unable to retrieve events in namespace `{}`.", namespace);

        EngineError::new(event_details, Tag::K8sGetLogs, message, Some(raw_error), None, None)
    }

    /// Creates new error for kubernetes describe.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `selector`: Selector to get description for.
    /// * `namespace`: Resource's namespace to get description for.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_describe(
        event_details: EventDetails,
        selector: String,
        namespace: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!(
            "Error, unable to describe pod with selector `{}` in namespace `{}`.",
            selector, namespace
        );

        EngineError::new(event_details, Tag::K8sDescribe, message, Some(raw_error), None, None)
    }

    /// Creates new error for kubernetes history.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `namespace`: Resource's namespace to get history for.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_history(event_details: EventDetails, namespace: String, raw_error: CommandError) -> EngineError {
        let message = format!("Error, unable to get history in namespace `{}`.", namespace);

        EngineError::new(event_details, Tag::K8sHistory, message, Some(raw_error), None, None)
    }

    /// Creates new error for kubernetes namespace creation issue.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `namespace`: Resource's namespace to get history for.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_create_namespace(
        event_details: EventDetails,
        namespace: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Error, unable to create namespace `{}`.", namespace);

        EngineError::new(
            event_details,
            Tag::K8sCannotCreateNamespace,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error for kubernetes pod not being ready.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `selector`: Selector to get description for.
    /// * `namespace`: Resource's namespace to get history for.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_pod_not_ready(
        event_details: EventDetails,
        selector: String,
        namespace: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!(
            "Error, pod with selector `{}` in namespace `{}` is not ready.",
            selector, namespace
        );

        EngineError::new(event_details, Tag::K8sPodIsNotReady, message, Some(raw_error), None, None)
    }

    /// Creates new error for kubernetes node not being ready with the requested version.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `requested_version`: Requested version.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_node_not_ready_with_requested_version(
        event_details: EventDetails,
        requested_version: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Error, node is not ready with the requested version `{}`.", requested_version);

        EngineError::new(
            event_details,
            Tag::K8sNodeIsNotReadyWithTheRequestedVersion,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error for kubernetes node not being ready.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_node_not_ready(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message = "Error, node is not ready.";

        EngineError::new(
            event_details,
            Tag::K8sNodeIsNotReady,
            message.to_string(),
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error for kubernetes validate required CPU and burstable.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `total_cpus_raw`: Total CPUs raw format.
    /// * `cpu_burst_raw`: CPU burst raw.
    /// * `raw_error`: Raw error message.
    pub fn new_k8s_validate_required_cpu_and_burstable_error(
        event_details: EventDetails,
        total_cpus_raw: String,
        cpu_burst_raw: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!(
            "Error while trying to validate required CPU ({}) and burstable ({}).",
            total_cpus_raw, cpu_burst_raw
        );

        EngineError::new(
            event_details,
            Tag::K8sValidateRequiredCPUandBurstableError,
            message,
            Some(raw_error),
            None,
            Some("Please ensure your configuration is valid.".to_string()),
        )
    }

    /// Creates new error for kubernetes not being able to get crash looping pods.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `missing_binary_name`: Name of the missing required binary.
    pub fn new_missing_required_binary(event_details: EventDetails, missing_binary_name: String) -> EngineError {
        let message = format!("`{}` binary is required but was not found.", missing_binary_name);

        EngineError::new(event_details, Tag::CannotFindRequiredBinary, message, None, None, None)
    }

    /// Creates new error for subnets count not being even. Subnets count should be even to get the same number as private and public.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `zone_name`: Number of subnets.
    /// * `subnets_count`: Number of subnets.
    pub fn new_subnets_count_is_not_even(
        event_details: EventDetails,
        zone_name: String,
        subnets_count: usize,
    ) -> EngineError {
        let message = format!(
            "Number of subnets for zone `{:?}` should be even but got `{}` subnets.",
            zone_name, subnets_count,
        );

        EngineError::new(event_details, Tag::SubnetsCountShouldBeEven, message, None, None, None)
    }

    /// Creates new error for IAM role which cannot be retrieved or created.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `role_name`: IAM role name which failed to be created.
    /// * `raw_error`: Raw error message.
    pub fn new_cannot_get_or_create_iam_role(
        event_details: EventDetails,
        role_name: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Error while getting or creating the role {}.", role_name,);

        EngineError::new(
            event_details,
            Tag::CannotGetOrCreateIamRole,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error for cannot generate and copy all files from a directory to another directory.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `from_dir`: Source directory for the copy.
    /// * `to_dir`: Target directory for the copy.
    /// * `raw_error`: Raw error message.
    pub fn new_cannot_copy_files_from_one_directory_to_another(
        event_details: EventDetails,
        from_dir: String,
        to_dir: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Error while trying to copy all files from `{}` to `{}`.", from_dir, to_dir);

        EngineError::new(
            event_details,
            Tag::CannotCopyFilesFromDirectoryToDirectory,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error for cannot pause cluster because some tasks are still running on the engine.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    pub fn new_cannot_pause_cluster_tasks_are_running(
        event_details: EventDetails,
        raw_error: Option<CommandError>,
    ) -> EngineError {
        let message = "Can't pause the infrastructure now, some engine jobs are currently running.";

        EngineError::new(
            event_details,
            Tag::CannotPauseClusterTasksAreRunning,
            message.to_string(),
            raw_error,
            None,
            None,
        )
    }

    /// Creates new error for terraform.
    /// Every single Terraform error raised in the engine should end-up here.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw Terraform error message.
    pub fn new_terraform_error(event_details: EventDetails, terraform_error: TerraformError) -> EngineError {
        // All Terraform issues are handled here.
        // TODO(benjaminch): Add some point, safe message should be moved inside Terraform impl directly
        match terraform_error {
            TerraformError::Unknown { .. } => EngineError::new(
                event_details,
                Tag::TerraformUnknownError,
                terraform_error.to_string(), // Note: end-game goal is to have 0 Unknown Terraform issues. Showing everything in this case is just more convenient for both user and Qovery team.
                Some(terraform_error.into()), // Note: Terraform error message are supposed to be safe
                None,
                Some(DEFAULT_HINT_MESSAGE.to_string()),
            ),
            TerraformError::MultipleInterruptsReceived { .. } => EngineError::new(
                event_details,
                Tag::TerraformMultipleInterruptsReceived,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()), // Note: Terraform error message are supposed to be safe
                None,
                Some(DEFAULT_HINT_MESSAGE.to_string()),
            ),
            TerraformError::InvalidCredentials { .. } => EngineError::new(
                event_details,
                Tag::TerraformInvalidCredentials,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()), // Note: Terraform error message are supposed to be safe
                None,
                Some(DEFAULT_HINT_MESSAGE.to_string()),
            ),
            TerraformError::AccountBlockedByProvider { .. } => {
                let hint_message = match event_details.provider_kind() {
                   Some(Kind::Aws) => Some("This AWS account is currently blocked and not recognized as a valid account. Please contact aws-verification@amazon.com directly to get more details. Maybe you are not allowed to use your free tier in this region? Maybe you need to provide billing info? ".to_string()),
                    _ => Some("This account is currently blocked by your cloud provider, please contact them directly.".to_string()),
                };

                EngineError::new(
                    event_details,
                    Tag::TerraformAccountBlockedByProvider,
                    terraform_error.to_safe_message(),
                    Some(terraform_error.into()), // Note: Terraform error message are supposed to be safe
                    Some(Url::parse("https://hub.qovery.com/docs/using-qovery/troubleshoot/#my-cloud-account-has-been-blocked-what-should-i-do").expect("Error while trying to parse error link helper for `Tag::TerraformAccountBlockedByProvider`, URL is not valid.")),
                    hint_message,
                )
            },
            TerraformError::ConfigFileNotFound { .. } => EngineError::new(
                event_details,
                Tag::TerraformConfigFileNotFound,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()), // Note: Terraform error message are supposed to be safe
                None,
                Some("This is normal if it's a newly created cluster".to_string()),
            ),
            TerraformError::ConfigFileInvalidContent { .. } => EngineError::new(
                event_details,
                Tag::TerraformConfigFileInvalidContent,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()), // Note: Terraform error message are supposed to be safe
                None,
                Some("Did you manually performed changes AWS side?".to_string()),
            ),
            TerraformError::CannotDeleteLockFile { .. } => EngineError::new(
                event_details,
                Tag::TerraformCannotDeleteLockFile,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()), // Note: Terraform error message are supposed to be safe
                None,
                None,
            ),
            TerraformError::CannotRemoveEntryOutOfStateList { .. } => EngineError::new(
                event_details,
                Tag::TerraformCannotRemoveEntryOut,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()), // Note: Terraform error message are supposed to be safe
                None,
                None,
            ),
            TerraformError::ContextUnsupportedParameterValue { .. } => EngineError::new(
                event_details,
                Tag::TerraformContextUnsupportedParameterValue,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()), // Note: Terraform error message are supposed to be safe
                None,
                None,
            ),
            TerraformError::QuotasExceeded {
                raw_message: ref _raw_message,
                ref sub_type,
            } => {
                let terraform_error_string = terraform_error.to_safe_message();
                match sub_type.clone() {
                    QuotaExceededError::ResourceLimitExceeded { resource_type, max_resource_count } => {
                        if let Some(Kind::Aws) = event_details.provider_kind() {
                            return EngineError::new(
                                event_details,
                                Tag::TerraformCloudProviderQuotasReached,
                                terraform_error_string,
                                Some(terraform_error.into()), // Note: Terraform error message are supposed to be safe
                                Some(Url::parse("https://hub.qovery.com/docs/using-qovery/troubleshoot/").expect("Error while trying to parse error link helper for `QuotaExceededError::ResourceLimitExceeded`, URL is not valid.")),
                                Some(format!("Request AWS to increase your `{}` limit{} via this page http://aws.amazon.com/contact-us/ec2-request.", resource_type, match max_resource_count {
                                    None => "".to_string(),
                                    Some(count) => format!(" (current max = {}", count),
                                })),
                            );
                        }

                        // No cloud provider specifics
                        EngineError::new(
                            event_details,
                            Tag::TerraformCloudProviderQuotasReached,
                            terraform_error_string, // Note: Terraform error message are supposed to be safe
                            Some(terraform_error.into()),
                            Some(Url::parse("https://hub.qovery.com/docs/using-qovery/troubleshoot/").expect("Error while trying to parse error link helper for `QuotaExceededError::ResourceLimitExceeded`, URL is not valid.")),
                            Some(format!("Request your cloud provider to increase your `{}` limit{}", resource_type, match max_resource_count {
                                None => "".to_string(),
                                Some(count) => format!("(current max = {}", count),
                            })),
                        )
                    },

                    // SCW specifics
                    QuotaExceededError::ScwNewAccountNeedsValidation => EngineError::new(
                        event_details,
                        Tag::TerraformCloudProviderActivationRequired,
                        terraform_error_string, // Note: Terraform error message are supposed to be safe
                        Some(terraform_error.into()),
                        Some(Url::parse("https://hub.qovery.com/docs/using-qovery/configuration/cloud-service-provider/scaleway/#connect-your-scaleway-account").expect("Error while trying to parse error link helper for `QuotaExceededError::ScwNewAccountNeedsValidation`, URL is not valid.")),
                        Some("If you have a new Scaleway account, your quota must be unlocked by the Scaleway support teams. To do this, open a ticket with their support with the following message: 'Hello, I would like to deploy my applications on Scaleway with Qovery. Can you increase my quota for the current Kubernetes node type to 10 please? '".to_string()),
                    ),
                }
            }
            TerraformError::ServiceNotActivatedOptInRequired { .. } => EngineError::new(
                event_details,
                Tag::TerraformServiceNotActivatedOptInRequired,
                terraform_error.to_safe_message(), // Note: Terraform error message are supposed to be safe
                Some(terraform_error.into()),
                None,
                None,
            ),
            TerraformError::AlreadyExistingResource { .. } => EngineError::new(
                event_details,
                Tag::TerraformAlreadyExistingResource,
                terraform_error.to_safe_message(), // Note: Terraform error message are supposed to be safe
                Some(terraform_error.into()),
                None,
                None,
            ),
            TerraformError::WaitingTimeoutResource { .. } => EngineError::new(
                event_details,
                Tag::TerraformWaitingTimeoutResource,
                terraform_error.to_safe_message(), // Note: Terraform error message are supposed to be safe
                Some(terraform_error.into()),
                None,
                None,
            ),
            TerraformError::NotEnoughPermissions { .. } => EngineError::new(
                event_details,
                Tag::TerraformNotEnoughPermissions,
                terraform_error.to_safe_message(), // Note: Terraform error message are supposed to be safe
                Some(terraform_error.into()),
                Some(Url::parse("https://hub.qovery.com/docs/getting-started/install-qovery/").expect("Error while trying to parse error link helper for `TerraformError::NotEnoughPermissions`, URL is not valid.")),
                Some("Make sure you provide proper credentials for your cloud account.".to_string()),
            ),
            TerraformError::WrongExpectedState { .. } => EngineError::new(
                event_details,
                Tag::TerraformWrongState,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()),
                None,
                Some("Try to set the resource in the desired state from your Cloud provider web console or API".to_string()),
            ),
            TerraformError::ResourceDependencyViolation { .. } => EngineError::new(
                event_details,
                Tag::TerraformWrongState,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()),
                None,
                None,
            ),
            TerraformError::InstanceTypeDoesntExist { .. } => EngineError::new(
                event_details,
                Tag::TerraformInstanceTypeDoesntExist,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()),
                None,
                Some("Select a different instance type in your cluster settings and re-launch the installation process".to_string()),
            ),
            TerraformError::InstanceVolumeCannotBeDownSized { .. } => EngineError::new(
                event_details,
                Tag::TerraformInstanceVolumeCannotBeReduced,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()),
                None,
                Some("An existing instance volume cannot be downsized, you can only increase its volume.".to_string()),
            ),
            TerraformError::InvalidCIDRBlock { .. } => EngineError::new(
                event_details,
                Tag::TerraformInvalidCIDRBlock,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()),
                None,
                Some("The CIDR block is equal to or more specific than one of this VPC's CIDR blocks.".to_string()),
            ),
            TerraformError::StateLocked { .. } => EngineError::new(
                event_details,
                Tag::TerraformStateLocked,
                terraform_error.to_safe_message(),
                Some(terraform_error.into()),
                None,
                Some("Your deployment failed because Terraform faced a state lock. Please contact Qovery team to get unlocked.".to_string()),
            ),
        }
    }

    /// Creates new error while setup Helm charts to deploy.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_helm_charts_setup_error(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message = "Error while helm charts setup";

        EngineError::new(
            event_details,
            Tag::HelmChartsSetupError,
            message.to_string(),
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error while deploying Helm charts.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_helm_charts_deploy_error(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message = "Error while helm charts deployment";

        EngineError::new(
            event_details,
            Tag::HelmChartsDeployError,
            message.to_string(),
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error while upgrading Helm charts.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_helm_charts_upgrade_error(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message = "Error while helm charts upgrade";

        EngineError::new(
            event_details,
            Tag::HelmChartsUpgradeError,
            message.to_string(),
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error from an Helm error
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `error`: Raw error message.
    pub fn new_container_registry_error(event_details: EventDetails, error: ContainerRegistryError) -> EngineError {
        match error {
            ContainerRegistryError::InvalidCredentials => EngineError::new(
                event_details,
                Tag::ContainerRegistryInvalidCredentials,
                "Container registry: credentials are not valid.".to_string(),
                Some(error.into()),
                Some(Url::parse("https://hub.qovery.com/docs/getting-started/install-qovery/").expect("Error while trying to parse error link helper for `ContainerRegistryError::InvalidCredentials`, URL is not valid.")),
                Some("Make sure you provide proper credentials for your cloud account.".to_string()),
            ),
            ContainerRegistryError::CannotGetCredentials => EngineError::new(
                event_details,
                Tag::ContainerRegistryCannotGetCredentials,
                "Container registry: cannot get credentials.".to_string(),
                Some(error.into()),
                None,
                None,
            ),
            ContainerRegistryError::CannotCreateRegistry { ref registry_name, .. } => EngineError::new(
                event_details,
                Tag::ContainerRegistryCannotCreateRegistry,
                format!("Container registry: cannot create registry: `{}`.", registry_name),
                Some(error.into()),
                None,
                None,
            ),
            ContainerRegistryError::CannotDeleteRegistry { ref registry_name, .. } => EngineError::new(
                event_details,
                Tag::ContainerRegistryCannotDeleteRegistry,
                format!("Container registry: cannot delete registry: `{}`.", registry_name),
                Some(error.into()),
                None,
                None,
            ),
            ContainerRegistryError::CannotDeleteImage { ref image_name, ref registry_name, ref repository_name, .. } => EngineError::new(
                event_details,
                Tag::ContainerRegistryCannotDeleteImage,
                format!("Container registry: cannot delete image `{}` from repository `{}` in registry `{}`.", image_name, repository_name, registry_name),
                Some(error.into()),
                None,
                None,
            ),
            ContainerRegistryError::ImageDoesntExistInRegistry { ref image_name, ref registry_name, ref repository_name, .. } => EngineError::new(
                event_details,
                Tag::ContainerRegistryImageDoesntExist,
                format!("Container registry: image `{}` doesn't exist in repository `{}` in registry `{}`.", image_name, repository_name, registry_name),
                Some(error.into()),
                None,
                None,
            ),
            ContainerRegistryError::RepositoryDoesntExistInRegistry { ref registry_name, ref repository_name, .. } => EngineError::new(
                event_details,
                Tag::ContainerRegistryRepositoryDoesntExistInRegistry,
                format!("Container registry: repository `{}` doesn't exist in registry `{}`.", repository_name, registry_name),
                Some(error.into()),
                None,
                None,
            ),
            ContainerRegistryError::RegistryDoesntExist { ref registry_name, .. } => EngineError::new(
                event_details,
                Tag::ContainerRegistryRegistryDoesntExist,
                format!("Container registry: registry `{}` doesn't exist.", registry_name),
                Some(error.into()),
                None,
                None,
            ),
            ContainerRegistryError::CannotLinkRegistryToCluster { ref registry_name, ref cluster_id, .. } => EngineError::new(
                event_details,
                Tag::ContainerRegistryCannotLinkRegistryToCluster,
                format!("Container registry: registry `{}` cannot be linked to cluster `{}`.", registry_name, cluster_id),
                Some(error.into()),
                None,
                None,
            ),
            ContainerRegistryError::CannotCreateRepository { ref registry_name, ref repository_name, .. } => EngineError::new(
                event_details,
                Tag::ContainerRegistryCannotCreateRepository,
                format!("Container registry: cannot create repository `{}` in registry `{}`.", repository_name, registry_name),
                Some(error.into()),
                None,
                None,
            ),
            ContainerRegistryError::CannotDeleteRepository { ref registry_name, ref repository_name, .. } => EngineError::new(
                event_details,
                Tag::ContainerRegistryCannotDeleteRepository,
                format!("Container registry: cannot delete repository `{}` from registry `{}`.", repository_name, registry_name),
                Some(error.into()),
                None,
                None,
            ),
            ContainerRegistryError::CannotSetRepositoryLifecyclePolicy { ref registry_name, ref repository_name, .. } => EngineError::new(
                event_details,
                Tag::ContainerRegistryCannotSetRepositoryLifecycle,
                format!("Container registry: cannot set lifetime on repository `{}` in registry `{}`.", repository_name, registry_name),
                Some(error.into()),
                None,
                None,
            ),
            ContainerRegistryError::CannotSetRepositoryTags { ref registry_name, ref repository_name, .. } => EngineError::new(event_details,
            Tag::ContainerRegistryCannotSetRepositoryTags,
            format!("Container registry: cannot set tags on repository `{}` in registry `{}`.", repository_name, registry_name),
            Some(error.into()),
                                                                                                                               None,
                                                                                                                               None,
            ),
            ContainerRegistryError::Unknown {..} => EngineError::new(event_details, Tag::ContainerRegistryUnknownError, "Container registry unknown error.".to_string(), Some(error.into()),None, None)
        }
    }

    /// Creates new error from an Build error
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `error`: Raw error message.
    pub fn new_build_error(event_details: EventDetails, error: BuildError, user_message: String) -> EngineError {
        let command_error = CommandError::from(error);

        EngineError::new(event_details, Tag::BuilderError, user_message, Some(command_error), None, None)
    }

    /// Creates new error from an Container Registry error
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `error`: Raw error message.
    pub fn new_helm_error(event_details: EventDetails, error: HelmError) -> EngineError {
        let cmd_error = match &error {
            HelmError::Killed(_, _) => return EngineError::new_task_cancellation_requested(event_details),
            HelmError::CmdError(_, _, cmd_error) => Some(cmd_error.clone()),
            _ => None,
        };

        let tag = match &error {
            HelmError::Timeout(_, _, _) => Tag::HelmDeployTimeout,
            _ => Tag::HelmChartsDeployError,
        };

        EngineError::new(event_details, tag, error.to_string(), cmd_error, None, None)
    }

    /// Creates new error while uninstalling Helm chart.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `helm_chart`: Helm chart name.
    /// * `raw_error`: Raw error message.
    pub fn new_helm_chart_uninstall_error(
        event_details: EventDetails,
        helm_chart: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Error while uninstalling helm chart: `{}`.", helm_chart);

        EngineError::new(
            event_details,
            Tag::HelmChartUninstallError,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error while trying to get Helm chart history.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `helm_chart`: Helm chart name.
    /// * `namespace`: Namespace.
    /// * `raw_error`: Raw error message.
    pub fn new_helm_chart_history_error(
        event_details: EventDetails,
        helm_chart: String,
        namespace: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!(
            "Error while trying to get helm chart `{}` history in namespace `{}`.",
            helm_chart, namespace
        );

        EngineError::new(event_details, Tag::HelmHistoryError, message, Some(raw_error), None, None)
    }

    /// Creates new error while trying to get any available VPC.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_cannot_get_any_available_vpc(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message = "Error while trying to get any available VPC.";

        EngineError::new(
            event_details,
            Tag::CannotGetAnyAvailableVPC,
            message.to_string(),
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error while trying to get supported versions.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `product_name`: Product name for which we want to get supported versions.
    /// * `raw_error`: Raw error message.
    pub fn new_cannot_get_supported_versions_error(
        event_details: EventDetails,
        product_name: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Error while trying to get supported versions for `{}`.", product_name);

        EngineError::new(
            event_details,
            Tag::CannotGetSupportedVersions,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new unsupported version error.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `product_name`: Product name for which version is not supported.
    /// * `version`: unsupported version raw string.
    pub fn new_unsupported_version_error(
        event_details: EventDetails,
        product_name: String,
        version: String,
    ) -> EngineError {
        let message = format!("Error, version `{}` is not supported for `{}`.", version, product_name);

        EngineError::new(event_details, Tag::UnsupportedVersion, message, None, None, None)
    }

    /// Creates new error while trying to get cluster.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_cannot_get_cluster_error(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message = "Error, cannot get cluster.";

        EngineError::new(
            event_details,
            Tag::CannotGetCluster,
            message.to_string(),
            Some(raw_error),
            None,
            Some("Maybe there is a lag and cluster is not yet reported, please retry later.".to_string()),
        )
    }

    /// Creates new error while trying to start a client service.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `service_id`: Client service ID.
    /// * `service_name`: Client service name.
    pub fn new_client_service_failed_to_start_error(
        event_details: EventDetails,
        service_id: String,
        service_name: String,
    ) -> EngineError {
        // TODO(benjaminch): Service should probably passed otherwise, either inside event_details or via a new dedicated struct.
        let message = format!("Service `{}` (id `{}`) failed to start. ⤬", service_name, service_id);

        EngineError::new(
            event_details,
            Tag::ClientServiceFailedToStart,
            message,
            None,
            None,
            Some("Ensure you can run it without issues with `qovery run` and check its logs from the web interface or the CLI with `qovery log`. \
                This issue often occurs due to ports misconfiguration. Make sure you exposed the correct port (using EXPOSE statement in Dockerfile or via Qovery configuration).".to_string()),
        )
    }

    /// Creates new error while trying to deploy a client service before start.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `service_id`: Client service ID.
    /// * `service_name`: Client service name.
    pub fn new_client_service_failed_to_deploy_before_start_error(
        event_details: EventDetails,
        service_id: String,
        service_name: String,
    ) -> EngineError {
        // TODO(benjaminch): Service should probably passed otherwise, either inside event_details or via a new dedicated struct.
        let message = format!(
            "Service `{}` (id `{}`) failed to deploy (before start).",
            service_name, service_id
        );

        EngineError::new(
            event_details,
            Tag::ClientServiceFailedToDeployBeforeStart,
            message,
            None,
            None,
            None,
        )
    }

    /// Creates new error while trying to start a client service before start.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `service_id`: Client service ID.
    /// * `service_type`: Client service type.
    /// * `raw_error`: Raw error message.
    pub fn new_database_failed_to_start_after_several_retries(
        event_details: EventDetails,
        service_id: String,
        service_type: String,
        raw_error: Option<CommandError>,
    ) -> EngineError {
        let message = format!(
            "Database `{}` (id `{}`) failed to start after several retries.",
            service_type, service_id
        );

        let hint = match service_type.eq("Redis") {
            true => Some("If you are redeploying a managed Redis v6 created before 2022-21-07, it means you're using a deprecated version. The database is running but we recommend to create a fresh new one to replace the actual.".to_string()),
            false => None
        };

        EngineError::new(
            event_details,
            Tag::DatabaseFailedToStartAfterSeveralRetries,
            message,
            raw_error,
            None,
            hint,
        )
    }

    /// Creates new error while trying to deploy a router.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    pub fn new_router_failed_to_deploy(event_details: EventDetails) -> EngineError {
        let message = "Router has failed to be deployed.";

        EngineError::new(event_details, Tag::RouterFailedToDeploy, message.to_string(), None, None, None)
    }

    /// Creates new error when trying to connect to user's account with its credentials.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    pub fn new_client_invalid_cloud_provider_credentials(event_details: EventDetails) -> EngineError {
        let message = "Your cloud provider account seems to be no longer valid (bad credentials).";

        EngineError::new(
            event_details,
            Tag::CloudProviderClientInvalidCredentials,
            message.to_string(),
            None,
            None,
            Some("Please contact your Organization administrator to fix or change the Credentials.".to_string()),
        )
    }

    /// Creates new error when trying to parse a version number.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_version_number`: Raw version number string.
    /// * `raw_error`: Raw error message.
    pub fn new_version_number_parsing_error(
        event_details: EventDetails,
        raw_version_number: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Error while trying to parse `{}` to a version number.", raw_version_number);

        EngineError::new(
            event_details,
            Tag::VersionNumberParsingError,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error while trying to get cluster.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_missing_workers_group_info_error(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message = "Error, cannot get cluster.";

        EngineError::new(
            event_details,
            Tag::CannotGetCluster,
            message.to_string(),
            Some(raw_error),
            None,
            Some("Maybe there is a lag and cluster is not yet reported, please retry later.".to_string()),
        )
    }

    /// No nodegroup information given from the cloud provider
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_missing_nodegroup_information_error(event_details: EventDetails) -> EngineError {
        let message = "Error from the cloud provider, missing Kubernetes nodegroup information";

        EngineError::new(
            event_details,
            Tag::CannotGetNodeGroupInfo,
            message.to_string(),
            None,
            None,
            None,
        )
    }

    /// Can't retrieve Cloud provider node group list
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_nodegroup_list_error(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message = "Error, cannot get Kubernetes nodegroup list from your cloud provider.";

        EngineError::new(
            event_details,
            Tag::CannotGetNodeGroupList,
            message.to_string(),
            Some(raw_error),
            None,
            None,
        )
    }

    /// No cluster found
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_no_cluster_found_error(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message = "Error, no cluster found.";

        EngineError::new(
            event_details,
            Tag::CannotGetCluster,
            message.to_string(),
            Some(raw_error),
            None,
            Some("Maybe there is a lag and cluster is not yet reported, please retry later.".to_string()),
        )
    }

    /// Too many clusters found, while expected only one
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_multiple_cluster_found_expected_one_error(
        event_details: EventDetails,
        raw_error: CommandError,
    ) -> EngineError {
        let message = "Too many clusters found with this name, where 1 was expected";

        EngineError::new(
            event_details,
            Tag::OnlyOneClusterExpected,
            message.to_string(),
            Some(raw_error),
            None,
            Some("Please contact Qovery support for investigation.".to_string()),
        )
    }

    /// Current task cancellation has been requested.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    pub fn new_task_cancellation_requested(event_details: EventDetails) -> EngineError {
        let message = "Task cancellation has been requested.";

        EngineError::new(
            event_details,
            Tag::TaskCancellationRequested,
            message.to_string(),
            None,
            None,
            None,
        )
    }

    /// Creates new error when trying to get Dockerfile.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `location_path`: Dockerfile location path.
    pub fn new_docker_cannot_find_dockerfile(event_details: EventDetails, location_path: String) -> EngineError {
        let message = format!("Dockerfile not found at location `{}`.", location_path);

        EngineError::new(
            event_details,
            Tag::BuilderDockerCannotFindAnyDockerfile,
            message,
            None,
            None,
            Some("Your Dockerfile is not present at the specified location, check your settings.".to_string()),
        )
    }

    /// Creates new error buildpack invalid language format.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `requested_language`: Requested language.
    pub fn new_buildpack_invalid_language_format(
        event_details: EventDetails,
        requested_language: String,
    ) -> EngineError {
        let message = format!("Cannot build: Invalid buildpacks language format: `{}`.", requested_language);

        EngineError::new(
            event_details,
            Tag::BuilderBuildpackInvalidLanguageFormat,
            message,
            None,
            None,
            Some("Expected format `builder[@version]`.".to_string()),
        )
    }

    /// Creates new error when trying to build container.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `container_image_name`: Container image name.
    /// * `builders`: Builders name list.
    /// * `raw_error`: Raw error message.
    pub fn new_buildpack_cannot_build_container_image(
        event_details: EventDetails,
        container_image_name: String,
        builders: Vec<String>,
        raw_error: CommandError,
    ) -> EngineError {
        let message = "Cannot find a builder to build application.";

        EngineError::new(
            event_details,
            Tag::BuilderBuildpackCannotBuildContainerImage,
            message.to_string(),
            Some(raw_error),
            None,
            Some(format!(
                "Qovery can't build your container image {} with one of the following builders: {}. Please do provide a valid Dockerfile to build your application or contact the support.",
                container_image_name,
                builders.join(", ")
            ),),
        )
    }

    /// Creates new error when trying to get build.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `commit_id`: Commit ID of build to be retrieved.
    /// * `raw_error`: Raw error message.
    pub fn new_builder_get_build_error(
        event_details: EventDetails,
        commit_id: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Error while trying to get build with commit ID: `{}`.", commit_id);

        EngineError::new(event_details, Tag::BuilderGetBuildError, message, Some(raw_error), None, None)
    }

    /// Creates new error when builder is trying to clone a git repository.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `repository_url`: Repository URL.
    /// * `raw_error`: Raw error message.
    pub fn new_builder_clone_repository_error(
        event_details: EventDetails,
        repository_url: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Error while cloning repository `{}`.", repository_url);

        EngineError::new(
            event_details,
            Tag::BuilderCloningRepositoryError,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error when something went wrong because it's not implemented.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    pub fn new_not_implemented_error(event_details: EventDetails) -> EngineError {
        let message = "Error, something went wrong because it's not implemented.";

        EngineError::new(event_details, Tag::NotImplementedError, message.to_string(), None, None, None)
    }

    /// Creates new error from an Docker error
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `error`: Raw error message.
    pub fn new_docker_error(event_details: EventDetails, error: DockerError) -> EngineError {
        // build command error from underlying error in order to have proper safe message.
        let command_error = CommandError::from(error);
        EngineError::new(
            event_details,
            Tag::DockerError,
            command_error.message_safe(),
            Some(command_error),
            None,
            None,
        )
    }

    /// Creates new error when trying to push a Docker image.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `image_name`: Docker image name.
    /// * `repository_url`: Repository URL.
    /// * `raw_error`: Raw error message.
    pub fn new_docker_push_image_error(
        event_details: EventDetails,
        image_name: String,
        repository_url: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!(
            "Error, trying to push Docker image `{}` to repository `{}`.",
            image_name, repository_url
        );

        EngineError::new(event_details, Tag::DockerPushImageError, message, Some(raw_error), None, None)
    }

    /// Creates new error when trying to pull a Docker image.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `image_name`: Docker image name.
    /// * `repository_url`: Repository URL.
    /// * `raw_error`: Raw error message.
    pub fn new_docker_pull_image_error(
        event_details: EventDetails,
        image_name: String,
        repository_url: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!(
            "Error, trying to pull Docker image `{}` from repository `{}`.",
            image_name, repository_url
        );

        EngineError::new(event_details, Tag::DockerPullImageError, message, Some(raw_error), None, None)
    }

    /// Creates new error when trying to read Dockerfile content.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `dockerfile_path`: Dockerfile path.
    /// * `raw_error`: Raw error message.
    pub fn new_docker_cannot_read_dockerfile(
        event_details: EventDetails,
        dockerfile_path: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Can't read Dockerfile `{}`.", dockerfile_path);

        EngineError::new(
            event_details,
            Tag::BuilderDockerCannotReadDockerfile,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error when trying to extract env vars from Dockerfile.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `dockerfile_path`: Dockerfile path.
    /// * `raw_error`: Raw error message.
    pub fn new_docker_cannot_extract_env_vars_from_dockerfile(
        event_details: EventDetails,
        dockerfile_path: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Can't extract ENV vars from Dockerfile `{}`.", dockerfile_path);

        EngineError::new(
            event_details,
            Tag::BuilderDockerCannotExtractEnvVarsFromDockerfile,
            message,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error when trying to build Docker container.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `container_image_name`: Container image name.
    /// * `raw_error`: Raw error message.
    pub fn new_docker_cannot_build_container_image(
        event_details: EventDetails,
        container_image_name: String,
        raw_error: CommandError,
    ) -> EngineError {
        let message = format!("Error while building container image `{}`.", container_image_name);

        EngineError::new(
            event_details,
            Tag::BuilderDockerCannotBuildContainerImage,
            message,
            Some(raw_error),
            None,
            Some("It looks like there is something wrong in your Dockerfile. Try building the application locally with `docker build --no-cache`.".to_string()),
        )
    }

    /// Creates new error when trying to list Docker images.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_docker_cannot_list_images(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message = "Error while trying to list docker images.";

        EngineError::new(
            event_details,
            Tag::BuilderDockerCannotListImages,
            message.to_string(),
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new object storage error.
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `object_storage_error`: Object storage error.
    pub fn new_object_storage_error(
        event_details: EventDetails,
        object_storage_error: ObjectStorageError,
    ) -> EngineError {
        match object_storage_error {
            ObjectStorageError::QuotasExceeded { .. } => {
                if event_details.provider_kind() == Some(Kind::Scw) {
                    // Scaleway specifics
                    return EngineError::new(
                        event_details,
                        Tag::ObjectStorageQuotaExceeded,
                        "Error: quotas exceeded while trying to perform operation on object storage.".to_string(),
                        Some(object_storage_error.into()),
                        Some(Url::parse("https://hub.qovery.com/docs/using-qovery/configuration/cloud-service-provider/scaleway/#connect-your-scaleway-account").expect("Error while trying to parse error link helper for SCW `ObjectStorageError::QuotasExceeded`, URL is not valid.")),
                        Some("If you have a new Scaleway account, your quota must be unlocked by the Scaleway support teams. To do this, open a ticket with their support with the following message: 'Hello, I would like to deploy my applications on Scaleway with Qovery. Can you increase my quota for the current Kubernetes node type to 10 please?'".to_string()),
                    );
                }

                // Default error, no cloud provider specifics
                EngineError::new(
                    event_details,
                    Tag::ObjectStorageQuotaExceeded,
                    "Error: quotas exceeded while trying to perform operation on object storage.".to_string(),
                    Some(object_storage_error.into()),
                    None,
                    Some("Contact your cloud provider support to increase your quotas.".to_string()),
                )
            }
            ObjectStorageError::InvalidBucketName { ref bucket_name, .. } => EngineError::new(
                event_details,
                Tag::ObjectStorageInvalidBucketName,
                format!("Error: bucket name `{}` is not valid.", bucket_name),
                Some(object_storage_error.into()),
                None,
                Some("Check your cloud provider documentation to know bucket naming rules.".to_string()),
            ),
            ObjectStorageError::CannotCreateBucket { ref bucket_name, .. } => EngineError::new(
                event_details,
                Tag::ObjectStorageCannotCreateBucket,
                format!("Error, cannot create object storage bucket `{}`.", bucket_name,),
                Some(object_storage_error.into()),
                None,
                None,
            ),
            ObjectStorageError::CannotDeleteBucket { ref bucket_name, .. } => EngineError::new(
                event_details,
                Tag::ObjectStorageCannotDeleteBucket,
                format!("Error, cannot delete object storage bucket `{}`.", bucket_name,),
                Some(object_storage_error.into()),
                None,
                None,
            ),
            ObjectStorageError::CannotEmptyBucket { ref bucket_name, .. } => EngineError::new(
                event_details,
                Tag::ObjectStorageCannotEmptyBucket,
                format!("Error while trying to empty object storage bucket `{}`.", bucket_name,),
                Some(object_storage_error.into()),
                None,
                None,
            ),
            ObjectStorageError::CannotTagBucket { ref bucket_name, .. } => EngineError::new(
                event_details,
                Tag::ObjectStorageCannotTagBucket,
                format!("Error while trying to tag object storage bucket `{}`.", bucket_name,),
                Some(object_storage_error.into()),
                None,
                None,
            ),
            ObjectStorageError::CannotActivateBucketVersioning { ref bucket_name, .. } => EngineError::new(
                event_details,
                Tag::ObjectStorageCannotActivateBucketVersioning,
                format!(
                    "Error while trying to activate versioning for object storage bucket `{}`.",
                    bucket_name,
                ),
                Some(object_storage_error.into()),
                None,
                None,
            ),
            ObjectStorageError::CannotGetObjectFile {
                ref bucket_name,
                ref file_name,
                ..
            } => EngineError::new(
                event_details,
                Tag::ObjectStorageCannotGetObjectFile,
                format!(
                    "Error, cannot get file `{}` from object storage bucket `{}`.",
                    file_name, bucket_name,
                ),
                Some(object_storage_error.into()),
                None,
                None,
            ),
            ObjectStorageError::CannotUploadFile {
                ref bucket_name,
                ref file_name,
                ..
            } => EngineError::new(
                event_details,
                Tag::ObjectStorageCannotPutFileIntoBucket,
                format!(
                    "Error, cannot put file `{}` into object storage bucket `{}`.",
                    file_name, bucket_name,
                ),
                Some(object_storage_error.into()),
                None,
                None,
            ),
            ObjectStorageError::CannotDeleteFile {
                ref bucket_name,
                ref file_name,
                ..
            } => EngineError::new(
                event_details,
                Tag::ObjectStorageCannotDeleteFileIntoBucket,
                format!(
                    "Error, cannot delete file `{}` into object storage bucket `{}`.",
                    file_name, bucket_name,
                ),
                Some(object_storage_error.into()),
                None,
                None,
            ),
        }
    }

    /// Creates new error when trying to connect to vault endpoint
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_vault_connection_error(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message_safe = "Couldn't connect to Vault secret manager".to_string();

        EngineError::new(
            event_details,
            Tag::VaultConnectionError,
            message_safe,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error when Vault secret couldn't be retrieved
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_vault_secret_could_not_be_retrieved(
        event_details: EventDetails,
        raw_error: CommandError,
    ) -> EngineError {
        let message_safe = "Vault secret couldn't be retrieved".to_string();

        EngineError::new(
            event_details,
            Tag::VaultSecretCouldNotBeRetrieved,
            message_safe,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error when Vault secret couldn't be created or updated
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_vault_secret_could_not_be_created_or_updated(
        event_details: EventDetails,
        raw_error: CommandError,
    ) -> EngineError {
        let message_safe = "Vault secret couldn't be created or updated".to_string();

        EngineError::new(
            event_details,
            Tag::VaultSecretCouldNotBeCreatedOrUpdated,
            message_safe,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error when Vault secret couldn't be deleted
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_vault_secret_could_not_be_deleted(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message_safe = "Vault secret couldn't be deleted".to_string();

        EngineError::new(
            event_details,
            Tag::VaultSecretCouldNotBeDeleted,
            message_safe,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error when creating ClusterSecrets
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_error_when_create_cluster_secrets(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message_safe = "Qovery error when manipulating ClusterSecrets".to_string();

        EngineError::new(
            event_details,
            Tag::ClusterSecretsManipulationError,
            message_safe,
            Some(raw_error),
            None,
            None,
        )
    }

    /// Creates new error when checking cloud provider information provided
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_error_on_cloud_provider_information(
        event_details: EventDetails,
        raw_error: CommandError,
    ) -> EngineError {
        let message_safe = "Invalid cloud provider information".to_string();

        EngineError::new(
            event_details,
            Tag::CloudProviderInformationError,
            message_safe,
            Some(raw_error),
            None,
            Some("Check your cloud provider information".to_string()),
        )
    }

    /// Creates new error when checking container registry information provided
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_error_on_container_registry_information(
        event_details: EventDetails,
        raw_error: CommandError,
    ) -> EngineError {
        let message_safe = "Invalid container registry information".to_string();

        EngineError::new(
            event_details,
            Tag::ContainerRegistryInvalidInformation,
            message_safe,
            Some(raw_error),
            None,
            Some("Check your container registry information".to_string()),
        )
    }

    /// Creates new error when checking DNS provider information provided
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    /// * `raw_error`: Raw error message.
    pub fn new_error_on_dns_provider_information(event_details: EventDetails, raw_error: CommandError) -> EngineError {
        let message_safe = "Invalid DNS provider information".to_string();

        EngineError::new(
            event_details,
            Tag::DnsProviderInformationError,
            message_safe,
            Some(raw_error),
            None,
            Some("Check your DNS provider information".to_string()),
        )
    }

    /// Creates new error when client DNS provider credentials are invalid
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    pub fn new_error_on_dns_provider_invalid_credentials(event_details: EventDetails) -> EngineError {
        let message_safe = "Invalid DNS provider credentials".to_string();

        EngineError::new(
            event_details,
            Tag::DnsProviderInvalidCredentials,
            message_safe,
            None,
            None,
            Some("Check your DNS provider credentials".to_string()),
        )
    }

    /// Creates new error when client DNS provider credentials are invalid
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    pub fn new_error_on_dns_provider_invalid_api_url(event_details: EventDetails) -> EngineError {
        let message_safe = "Invalid DNS provider api url".to_string();

        EngineError::new(
            event_details,
            Tag::DnsProviderInvalidApiUrl,
            message_safe,
            None,
            None,
            Some("Check your DNS provider api url".to_string()),
        )
    }

    /// Creates new error to match Cloud Provider best practices
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    pub fn new_error_do_not_respect_cloud_provider_best_practices(
        event_details: EventDetails,
        raw_error: CommandError,
        url: Option<Url>,
    ) -> EngineError {
        EngineError::new(
            event_details,
            Tag::DoNotRespectCloudProviderBestPractices,
            raw_error.message_safe.clone(),
            Some(raw_error),
            url,
            None,
        )
    }

    /// Creates new error when getting load balancers from the cloud provider
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    pub fn new_cloud_provider_error_getting_load_balancers(
        event_details: EventDetails,
        cloud_provider_error_message: CommandError,
    ) -> EngineError {
        let message_safe = "Error while getting Load balancers from the cloud provider API".to_string();

        EngineError::new(
            event_details,
            Tag::CloudProviderGetLoadBalancer,
            message_safe,
            Some(cloud_provider_error_message),
            None,
            Some("Please ensure Qovery has correct permissions or try again later".to_string()),
        )
    }

    /// Creates new error when getting load balancer tags from the cloud provider
    ///
    /// Arguments:
    ///
    /// * `event_details`: Error linked event details.
    pub fn new_cloud_provider_error_getting_load_balancer_tags(
        event_details: EventDetails,
        cloud_provider_error_message: CommandError,
    ) -> EngineError {
        let message_safe = "Error while getting Load balancer tags from the cloud provider API".to_string();

        EngineError::new(
            event_details,
            Tag::CloudProviderGetLoadBalancerTags,
            message_safe,
            Some(cloud_provider_error_message),
            None,
            Some("Please ensure Qovery has correct permissions or try again later".to_string()),
        )
    }

    pub(crate) fn new_cloud_provider_error_deleting_load_balancer(
        event_details: EventDetails,
        cloud_provider_error_message: CommandError,
    ) -> EngineError {
        let message_safe = "Error while deleting Load balancer from the cloud provider API".to_string();

        EngineError::new(
            event_details,
            Tag::CloudProviderDeleteLoadBalancer,
            message_safe,
            Some(cloud_provider_error_message),
            None,
            None,
        )
    }
}
impl Display for EngineError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // Note: just in case, env vars are not leaked since it can hold sensitive data such as secrets.
        f.write_str(self.message(ErrorMessageVerbosity::FullDetailsWithoutEnvVars).as_str())
    }
}

#[cfg(test)]
mod tests {
    use crate::cloud_provider::Kind;
    use crate::errors::{CommandError, EngineError, ErrorMessageVerbosity};
    use crate::events::{EventDetails, InfrastructureStep, Stage, Transmitter};
    use crate::io_models::QoveryIdentifier;
    use uuid::Uuid;

    #[test]
    fn test_command_error_test_hidding_env_vars_in_message_safe_only() {
        // setup:
        let command_err = CommandError::new(
            "my safe message".to_string(),
            Some("my raw message".to_string()),
            Some(vec![("my_secret".to_string(), "my_secret_value".to_string())]),
        );

        // execute:
        let res = command_err.message(ErrorMessageVerbosity::SafeOnly);

        // verify:
        assert!(!res.contains("my_secret"));
        assert!(!res.contains("my_secret_value"));
    }

    #[test]
    fn test_command_error_test_hidding_env_vars_in_message_full_without_env_vars() {
        // setup:
        let command_err = CommandError::new(
            "my safe message".to_string(),
            Some("my raw message".to_string()),
            Some(vec![("my_secret".to_string(), "my_secret_value".to_string())]),
        );

        // execute:
        let res = command_err.message(ErrorMessageVerbosity::FullDetailsWithoutEnvVars);

        // verify:
        assert!(!res.contains("my_secret"));
        assert!(!res.contains("my_secret_value"));
    }

    #[test]
    fn test_engine_error_test_hidding_env_vars_in_message_safe_only() {
        // setup:
        let command_err = CommandError::new(
            "my safe message".to_string(),
            Some("my raw message".to_string()),
            Some(vec![("my_secret".to_string(), "my_secret_value".to_string())]),
        );
        let cluster_id = QoveryIdentifier::new_random();
        let engine_err = EngineError::new_unknown(
            EventDetails::new(
                Some(Kind::Scw),
                QoveryIdentifier::new_random(),
                QoveryIdentifier::new_random(),
                Uuid::new_v4().to_string(),
                Stage::Infrastructure(InfrastructureStep::Create),
                Transmitter::Kubernetes(Uuid::new_v4(), cluster_id.to_string()),
            ),
            "user_log_message".to_string(),
            Some(command_err),
            None,
            None,
        );

        // execute:
        let res = engine_err.message(ErrorMessageVerbosity::SafeOnly);

        // verify:
        assert!(!res.contains("my_secret"));
        assert!(!res.contains("my_secret_value"));
    }

    #[test]
    fn test_engine_error_test_hidding_env_vars_in_message_full_without_env_vars() {
        // setup:
        let command_err = CommandError::new(
            "my safe message".to_string(),
            Some("my raw message".to_string()),
            Some(vec![("my_secret".to_string(), "my_secret_value".to_string())]),
        );
        let cluster_id = QoveryIdentifier::new_random();
        let engine_err = EngineError::new_unknown(
            EventDetails::new(
                Some(Kind::Scw),
                QoveryIdentifier::new_random(),
                QoveryIdentifier::new_random(),
                Uuid::new_v4().to_string(),
                Stage::Infrastructure(InfrastructureStep::Create),
                Transmitter::Kubernetes(Uuid::new_v4(), cluster_id.to_string()),
            ),
            "user_log_message".to_string(),
            Some(command_err),
            None,
            None,
        );

        // execute:
        let res = engine_err.message(ErrorMessageVerbosity::SafeOnly);

        // verify:
        assert!(!res.contains("my_secret"));
        assert!(!res.contains("my_secret_value"));
    }

    #[test]
    fn test_command_error_test_hidding_env_vars_in_debug() {
        // setup:
        let command_err = CommandError::new(
            "my safe message".to_string(),
            Some("my raw message".to_string()),
            Some(vec![("my_secret".to_string(), "my_secret_value".to_string())]),
        );

        // execute:
        let res = format!("{}", command_err);

        // verify:
        assert!(!res.contains("my_secret"));
        assert!(!res.contains("my_secret_value"));
    }

    #[test]
    fn test_engine_error_test_hidding_env_vars_in_debug() {
        // setup:
        let command_err = CommandError::new(
            "my safe message".to_string(),
            Some("my raw message".to_string()),
            Some(vec![("my_secret".to_string(), "my_secret_value".to_string())]),
        );
        let cluster_id = QoveryIdentifier::new_random();
        let engine_err = EngineError::new_unknown(
            EventDetails::new(
                Some(Kind::Scw),
                QoveryIdentifier::new_random(),
                QoveryIdentifier::new_random(),
                "".to_string(),
                Stage::Infrastructure(InfrastructureStep::Create),
                Transmitter::Kubernetes(Uuid::new_v4(), cluster_id.to_string()),
            ),
            "user_log_message".to_string(),
            Some(command_err),
            None,
            None,
        );

        // execute:
        let res = format!("{:?}", engine_err);

        // verify:
        assert!(!res.contains("my_secret"));
        assert!(!res.contains("my_secret_value"));
    }

    #[test]
    fn should_clone_engine_error_with_a_different_stage() {
        // setup:
        let command_err = CommandError::new(
            "my safe message".to_string(),
            Some("my raw message".to_string()),
            Some(vec![("my_secret".to_string(), "my_secret_value".to_string())]),
        );
        let cluster_id = QoveryIdentifier::new_random();
        let engine_err = EngineError::new_unknown(
            EventDetails::new(
                Some(Kind::Scw),
                QoveryIdentifier::new_random(),
                QoveryIdentifier::new_random(),
                "".to_string(),
                Stage::Infrastructure(InfrastructureStep::Create),
                Transmitter::Kubernetes(Uuid::new_v4(), cluster_id.to_string()),
            ),
            "user_log_message".to_string(),
            Some(command_err),
            None,
            None,
        );

        // execute:
        let engine_error_with_terminated_stage =
            engine_err.clone_engine_error_with_stage(Stage::Infrastructure(InfrastructureStep::CreateError));

        // verify:
        assert_eq!(
            engine_error_with_terminated_stage.event_details.stage(),
            &Stage::Infrastructure(InfrastructureStep::CreateError)
        );
    }
}
