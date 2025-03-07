use crate::helpers::scaleway::{SCW_KUBERNETES_MAJOR_VERSION, SCW_KUBERNETES_MINOR_VERSION};
use crate::helpers::utilities::{context_for_cluster, engine_run_test, generate_cluster_id, generate_id, logger};
use ::function_name::named;
use qovery_engine::cloud_provider::aws::kubernetes::VpcQoveryNetworkMode;
use qovery_engine::cloud_provider::kubernetes::Kind as KKind;

use crate::helpers::common::ClusterDomain;
use crate::helpers::kubernetes::{cluster_test, ClusterTestType};
use qovery_engine::cloud_provider::Kind;
use qovery_engine::models::scaleway::ScwZone;
use qovery_engine::utilities::to_short_id;

#[cfg(feature = "test-scw-infra")]
fn create_and_destroy_kapsule_cluster(
    zone: ScwZone,
    test_type: ClusterTestType,
    major_boot_version: u8,
    minor_boot_version: u8,
    test_name: &str,
    vpc_network_mode: Option<VpcQoveryNetworkMode>,
) {
    engine_run_test(|| {
        let cluster_id = generate_cluster_id(zone.as_str());
        cluster_test(
            test_name,
            Kind::Scw,
            KKind::ScwKapsule,
            context_for_cluster(generate_id(), cluster_id),
            logger(),
            zone.as_str(),
            None,
            test_type,
            major_boot_version,
            minor_boot_version,
            &ClusterDomain::Default {
                cluster_id: to_short_id(&cluster_id),
            },
            vpc_network_mode,
            None,
        )
    })
}

#[cfg(feature = "test-scw-infra")]
#[named]
#[ignore]
#[test]
fn create_and_destroy_kapsule_cluster_par_1() {
    let zone = ScwZone::Paris1;
    create_and_destroy_kapsule_cluster(
        zone,
        ClusterTestType::Classic,
        SCW_KUBERNETES_MAJOR_VERSION,
        SCW_KUBERNETES_MINOR_VERSION,
        function_name!(),
        None,
    );
}

#[cfg(feature = "test-scw-infra")]
#[named]
#[test]
fn create_and_destroy_kapsule_cluster_par_2() {
    let zone = ScwZone::Paris2;
    create_and_destroy_kapsule_cluster(
        zone,
        ClusterTestType::Classic,
        SCW_KUBERNETES_MAJOR_VERSION,
        SCW_KUBERNETES_MINOR_VERSION,
        function_name!(),
        None,
    );
}

#[cfg(feature = "test-scw-infra")]
#[named]
#[test]
fn create_pause_and_destroy_kapsule_cluster_ams_1() {
    let zone = ScwZone::Amsterdam1;
    create_and_destroy_kapsule_cluster(
        zone,
        ClusterTestType::WithPause,
        SCW_KUBERNETES_MAJOR_VERSION,
        SCW_KUBERNETES_MINOR_VERSION,
        function_name!(),
        None,
    );
}

#[cfg(feature = "test-scw-infra")]
#[named]
#[test]
fn create_and_destroy_kapsule_cluster_war_1() {
    let zone = ScwZone::Warsaw1;
    create_and_destroy_kapsule_cluster(
        zone,
        ClusterTestType::Classic,
        SCW_KUBERNETES_MAJOR_VERSION,
        SCW_KUBERNETES_MINOR_VERSION,
        function_name!(),
        None,
    );
}

// only enable this test manually when we want to perform and validate upgrade process
#[cfg(feature = "test-scw-infra")]
#[test]
#[named]
#[ignore]
fn create_upgrade_and_destroy_kapsule_cluster_in_par_1() {
    let zone = ScwZone::Paris1;
    create_and_destroy_kapsule_cluster(
        zone,
        ClusterTestType::WithUpgrade,
        SCW_KUBERNETES_MAJOR_VERSION,
        SCW_KUBERNETES_MINOR_VERSION,
        function_name!(),
        None,
    );
}

// only enable this test manually when we want to perform and validate upgrade process
#[cfg(feature = "test-scw-infra")]
#[test]
#[named]
#[ignore]
fn create_upgrade_and_destroy_kapsule_cluster_in_par_2() {
    let zone = ScwZone::Paris2;
    create_and_destroy_kapsule_cluster(
        zone,
        ClusterTestType::WithUpgrade,
        SCW_KUBERNETES_MAJOR_VERSION,
        SCW_KUBERNETES_MINOR_VERSION,
        function_name!(),
        None,
    );
}

// only enable this test manually when we want to perform and validate upgrade process
#[cfg(feature = "test-scw-infra")]
#[test]
#[named]
#[ignore]
fn create_upgrade_and_destroy_kapsule_cluster_in_ams_1() {
    let zone = ScwZone::Amsterdam1;
    create_and_destroy_kapsule_cluster(
        zone,
        ClusterTestType::WithUpgrade,
        SCW_KUBERNETES_MAJOR_VERSION,
        SCW_KUBERNETES_MINOR_VERSION,
        function_name!(),
        None,
    );
}

// only enable this test manually when we want to perform and validate upgrade process
#[cfg(feature = "test-scw-infra")]
#[test]
#[named]
#[ignore]
fn create_upgrade_and_destroy_kapsule_cluster_in_war_1() {
    let zone = ScwZone::Warsaw1;
    create_and_destroy_kapsule_cluster(
        zone,
        ClusterTestType::WithUpgrade,
        SCW_KUBERNETES_MAJOR_VERSION,
        SCW_KUBERNETES_MINOR_VERSION,
        function_name!(),
        None,
    );
}
