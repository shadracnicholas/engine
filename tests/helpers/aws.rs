extern crate serde;
extern crate serde_derive;

use const_format::formatcp;
use qovery_engine::cloud_provider::aws::kubernetes::{Options, VpcQoveryNetworkMode};
use qovery_engine::cloud_provider::aws::regions::AwsRegion;
use qovery_engine::cloud_provider::aws::AWS;
use qovery_engine::cloud_provider::kubernetes::Kind as KubernetesKind;
use qovery_engine::cloud_provider::models::NodeGroups;
use qovery_engine::cloud_provider::qovery::EngineLocation;
use qovery_engine::cloud_provider::{CloudProvider, TerraformStateCredentials};
use qovery_engine::container_registry::ecr::ECR;
use qovery_engine::engine::InfrastructureContext;
use qovery_engine::io_models::context::Context;
use qovery_engine::logger::Logger;
use std::str::FromStr;
use std::sync::Arc;
use tracing::error;
use uuid::Uuid;

use crate::helpers::common::{Cluster, ClusterDomain};
use crate::helpers::dns::dns_provider_qoverydns;
use crate::helpers::kubernetes::{get_environment_test_kubernetes, KUBERNETES_MAX_NODES, KUBERNETES_MIN_NODES};
use crate::helpers::utilities::{build_platform_local_docker, FuncTestsSecrets};

pub const AWS_REGION_FOR_S3: AwsRegion = AwsRegion::EuWest3;
pub const AWS_TEST_REGION: AwsRegion = AwsRegion::EuWest3;
pub const AWS_KUBERNETES_MAJOR_VERSION: u8 = 1;
pub const AWS_KUBERNETES_MINOR_VERSION: u8 = 21;
pub const AWS_KUBERNETES_VERSION: &str = formatcp!("{}.{}", AWS_KUBERNETES_MAJOR_VERSION, AWS_KUBERNETES_MINOR_VERSION);
pub const AWS_DATABASE_INSTANCE_TYPE: &str = "db.t3.micro";
pub const AWS_DATABASE_DISK_TYPE: &str = "gp2";
pub const AWS_RESOURCE_TTL_IN_SECONDS: u32 = 14400;
pub const K3S_KUBERNETES_MAJOR_VERSION: u8 = 1;
pub const K3S_KUBERNETES_MINOR_VERSION: u8 = 23;

pub fn container_registry_ecr(context: &Context, logger: Box<dyn Logger>) -> ECR {
    let secrets = FuncTestsSecrets::new();
    if secrets.AWS_ACCESS_KEY_ID.is_none()
        || secrets.AWS_SECRET_ACCESS_KEY.is_none()
        || secrets.AWS_DEFAULT_REGION.is_none()
    {
        error!("Please check your Vault connectivity (token/address) or AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY/AWS_DEFAULT_REGION envrionment variables are set");
        std::process::exit(1)
    }

    ECR::new(
        context.clone(),
        "default-ecr-registry-Qovery Test",
        Uuid::new_v4(),
        "ea59qe62xaw3wjai",
        secrets.AWS_ACCESS_KEY_ID.unwrap().as_str(),
        secrets.AWS_SECRET_ACCESS_KEY.unwrap().as_str(),
        secrets.AWS_DEFAULT_REGION.unwrap().as_str(),
        logger,
        hashmap! {},
    )
    .unwrap()
}

pub fn aws_default_infra_config(context: &Context, logger: Box<dyn Logger>) -> InfrastructureContext {
    AWS::docker_cr_engine(
        context,
        logger,
        AWS_TEST_REGION.to_string().as_str(),
        KubernetesKind::Eks,
        AWS_KUBERNETES_VERSION.to_string(),
        &ClusterDomain::Default {
            cluster_id: context.cluster_short_id().to_string(),
        },
        None,
        KUBERNETES_MIN_NODES,
        KUBERNETES_MAX_NODES,
        EngineLocation::ClientSide,
    )
}

impl Cluster<AWS, Options> for AWS {
    fn docker_cr_engine(
        context: &Context,
        logger: Box<dyn Logger>,
        localisation: &str,
        kubernetes_kind: KubernetesKind,
        kubernetes_version: String,
        cluster_domain: &ClusterDomain,
        vpc_network_mode: Option<VpcQoveryNetworkMode>,
        min_nodes: i32,
        max_nodes: i32,
        engine_location: EngineLocation,
    ) -> InfrastructureContext {
        // use ECR
        let container_registry = Box::new(container_registry_ecr(context, logger.clone()));

        // use LocalDocker
        let build_platform = Box::new(build_platform_local_docker(context));

        // use AWS
        let cloud_provider: Arc<Box<dyn CloudProvider>> = Arc::new(AWS::cloud_provider(context, kubernetes_kind));
        let dns_provider = Arc::new(dns_provider_qoverydns(context, cluster_domain));

        let kubernetes = get_environment_test_kubernetes(
            context,
            cloud_provider.clone(),
            kubernetes_version.as_str(),
            dns_provider.clone(),
            logger.clone(),
            localisation,
            vpc_network_mode,
            min_nodes,
            max_nodes,
            engine_location,
        );

        InfrastructureContext::new(
            context.clone(),
            build_platform,
            container_registry,
            cloud_provider,
            dns_provider,
            kubernetes,
        )
    }

    fn cloud_provider(context: &Context, kubernetes_kind: KubernetesKind) -> Box<AWS> {
        let secrets = FuncTestsSecrets::new();
        let aws_region =
            AwsRegion::from_str(secrets.AWS_DEFAULT_REGION.unwrap().as_str()).expect("AWS region not supported");
        Box::new(AWS::new(
            context.clone(),
            Uuid::new_v4(),
            secrets
                .AWS_TEST_ORGANIZATION_ID
                .as_ref()
                .expect("AWS_TEST_ORGANIZATION_ID is not set")
                .as_str(),
            secrets
                .AWS_ACCESS_KEY_ID
                .expect("AWS_ACCESS_KEY_ID is not set")
                .as_str(),
            secrets
                .AWS_SECRET_ACCESS_KEY
                .expect("AWS_SECRET_ACCESS_KEY is not set")
                .as_str(),
            aws_region.to_aws_format(),
            aws_region.get_zones_to_string(),
            kubernetes_kind,
            TerraformStateCredentials {
                access_key_id: secrets
                    .TERRAFORM_AWS_ACCESS_KEY_ID
                    .expect("TERRAFORM_AWS_ACCESS_KEY_ID is n ot set"),
                secret_access_key: secrets
                    .TERRAFORM_AWS_SECRET_ACCESS_KEY
                    .expect("TERRAFORM_AWS_SECRET_ACCESS_KEY is not set"),
                region: "eu-west-3".to_string(),
            },
        ))
    }

    fn kubernetes_nodes(min_nodes: i32, max_nodes: i32) -> Vec<NodeGroups> {
        vec![
            NodeGroups::new("groupeks0".to_string(), min_nodes, max_nodes, "t3a.large".to_string(), 100)
                .expect("Problem while setup EKS nodes"),
        ]
    }

    fn kubernetes_cluster_options(
        secrets: FuncTestsSecrets,
        _cluster_id: Option<String>,
        engine_location: EngineLocation,
    ) -> Options {
        Options {
            ec2_zone_a_subnet_blocks: vec!["10.0.0.0/20".to_string(), "10.0.16.0/20".to_string()],
            ec2_zone_b_subnet_blocks: vec!["10.0.32.0/20".to_string(), "10.0.48.0/20".to_string()],
            ec2_zone_c_subnet_blocks: vec!["10.0.64.0/20".to_string(), "10.0.80.0/20".to_string()],
            eks_zone_a_subnet_blocks: vec!["10.0.0.0/20".to_string(), "10.0.16.0/20".to_string()],
            eks_zone_b_subnet_blocks: vec!["10.0.32.0/20".to_string(), "10.0.48.0/20".to_string()],
            eks_zone_c_subnet_blocks: vec!["10.0.64.0/20".to_string(), "10.0.80.0/20".to_string()],
            rds_zone_a_subnet_blocks: vec![
                "10.0.214.0/23".to_string(),
                "10.0.216.0/23".to_string(),
                "10.0.218.0/23".to_string(),
                "10.0.220.0/23".to_string(),
                "10.0.222.0/23".to_string(),
                "10.0.224.0/23".to_string(),
            ],
            rds_zone_b_subnet_blocks: vec![
                "10.0.226.0/23".to_string(),
                "10.0.228.0/23".to_string(),
                "10.0.230.0/23".to_string(),
                "10.0.232.0/23".to_string(),
                "10.0.234.0/23".to_string(),
                "10.0.236.0/23".to_string(),
            ],
            rds_zone_c_subnet_blocks: vec![
                "10.0.238.0/23".to_string(),
                "10.0.240.0/23".to_string(),
                "10.0.242.0/23".to_string(),
                "10.0.244.0/23".to_string(),
                "10.0.246.0/23".to_string(),
                "10.0.248.0/23".to_string(),
            ],
            documentdb_zone_a_subnet_blocks: vec![
                "10.0.196.0/23".to_string(),
                "10.0.198.0/23".to_string(),
                "10.0.200.0/23".to_string(),
            ],
            documentdb_zone_b_subnet_blocks: vec![
                "10.0.202.0/23".to_string(),
                "10.0.204.0/23".to_string(),
                "10.0.206.0/23".to_string(),
            ],
            documentdb_zone_c_subnet_blocks: vec![
                "10.0.208.0/23".to_string(),
                "10.0.210.0/23".to_string(),
                "10.0.212.0/23".to_string(),
            ],
            elasticache_zone_a_subnet_blocks: vec!["10.0.172.0/23".to_string(), "10.0.174.0/23".to_string()],
            elasticache_zone_b_subnet_blocks: vec!["10.0.176.0/23".to_string(), "10.0.178.0/23".to_string()],
            elasticache_zone_c_subnet_blocks: vec!["10.0.180.0/23".to_string(), "10.0.182.0/23".to_string()],
            elasticsearch_zone_a_subnet_blocks: vec!["10.0.184.0/23".to_string(), "10.0.186.0/23".to_string()],
            elasticsearch_zone_b_subnet_blocks: vec!["10.0.188.0/23".to_string(), "10.0.190.0/23".to_string()],
            elasticsearch_zone_c_subnet_blocks: vec!["10.0.192.0/23".to_string(), "10.0.194.0/23".to_string()],
            vpc_qovery_network_mode: VpcQoveryNetworkMode::WithoutNatGateways,
            vpc_cidr_block: "10.0.0.0/16".to_string(),
            eks_cidr_subnet: "20".to_string(),
            ec2_cidr_subnet: "20".to_string(),
            vpc_custom_routing_table: vec![],
            eks_access_cidr_blocks: secrets
                .EKS_ACCESS_CIDR_BLOCKS
                .as_ref()
                .unwrap()
                .replace('\"', "")
                .replace('[', "")
                .replace(']', "")
                .split(',')
                .map(|c| c.to_string())
                .collect(),
            ec2_access_cidr_blocks: secrets
                .EKS_ACCESS_CIDR_BLOCKS // FIXME ? use an EC2_ACCESS_CIDR_BLOCKS?
                .unwrap()
                .replace('\"', "")
                .replace('[', "")
                .replace(']', "")
                .split(',')
                .map(|c| c.to_string())
                .collect(),
            rds_cidr_subnet: "23".to_string(),
            documentdb_cidr_subnet: "23".to_string(),
            elasticache_cidr_subnet: "23".to_string(),
            elasticsearch_cidr_subnet: "23".to_string(),
            qovery_api_url: secrets.QOVERY_API_URL.unwrap(),
            qovery_engine_location: engine_location,
            engine_version_controller_token: secrets.QOVERY_ENGINE_CONTROLLER_TOKEN.unwrap(),
            agent_version_controller_token: secrets.QOVERY_AGENT_CONTROLLER_TOKEN.unwrap(),
            grafana_admin_user: "admin".to_string(),
            grafana_admin_password: "qovery".to_string(),
            discord_api_key: secrets.DISCORD_API_URL.unwrap(),
            qovery_nats_url: secrets.QOVERY_NATS_URL.unwrap(),
            qovery_ssh_key: secrets.QOVERY_SSH_USER.unwrap(),
            qovery_nats_user: secrets.QOVERY_NATS_USERNAME.unwrap(),
            qovery_nats_password: secrets.QOVERY_NATS_PASSWORD.unwrap(),
            tls_email_report: secrets.LETS_ENCRYPT_EMAIL_REPORT.unwrap(),
            qovery_grpc_url: secrets.QOVERY_GRPC_URL.unwrap(),
            jwt_token: secrets.QOVERY_CLUSTER_JWT_TOKEN.unwrap(),
            user_ssh_keys: vec![],
            user_network_config: None,
        }
    }
}
