use crate::helm::{
    application_context, chart_path, container_context, container_database_context, job_context, kubeconfig_path,
    lib_dir, managed_database_context,
};
use kube::core::DynamicObject;
use qovery_engine::cloud_provider::helm::CommonChart;
use qovery_engine::cloud_provider::helm::{ChartInfo, HelmAction, HelmChartNamespaces};
use qovery_engine::cmd::helm::Helm;
use qovery_engine::deployment_action::deploy_helm::HelmDeployment;
use qovery_engine::events::EventDetails;
use std::collections::HashMap;
use std::fs::{read_dir, File};
use std::path::{Path, PathBuf};
use std::{env, fs};
use tera::Context;

use super::router_context;

fn to_kube_kind(file_path: &str) -> DynamicObject {
    let file = File::open(file_path).unwrap_or_else(|_| panic!("Unable to open file {}", &file_path));
    let obj: DynamicObject =
        serde_yaml::from_reader(file).unwrap_or_else(|_| panic!("Unable to parse file {}", &file_path));
    obj
}

fn generate_template(chart_info: &ChartInfo) -> String {
    let home_dir = env::var("WORKSPACE_ROOT_DIR").expect("Missing environment variable WORKSPACE_ROOT_DIR");
    let template_dir = format!("{}/.qovery-workspace/rendered", home_dir);
    if !Path::new(&template_dir).exists() {
        let _ = fs::create_dir(template_dir.clone());
    }

    let helm = Helm::new(kubeconfig_path(), &[]).unwrap_or_else(|_| panic!("Unable to generate Helm struct"));
    let _ = helm.template_validate(chart_info, &[], Some(template_dir.as_str()));
    template_dir
}

fn get_kube_resources(
    chart_original_path: &str,
    chart_info: ChartInfo,
    render_custom_values_file: Option<PathBuf>,
    context: Context,
    event_details: EventDetails,
) -> HashMap<String, DynamicObject> {
    let helm_deployment = HelmDeployment::new(
        event_details,
        context,
        chart_original_path.parse().unwrap(),
        render_custom_values_file,
        chart_info.clone(),
    );
    let _ = helm_deployment.prepare_helm_chart();

    let template_dir = generate_template(&chart_info);

    let templates_path = format!("/{}/{}/templates", template_dir, &chart_info.name);
    let files = read_dir(&templates_path).unwrap_or_else(|_| panic!("Unable to read files in {}", &templates_path));
    let mut kube_resources: HashMap<String, DynamicObject> = HashMap::new();
    for file in files {
        let file_path = file
            .as_ref()
            .unwrap_or_else(|_| panic!("Unable to get file {:?}", &file))
            .path();
        let file_path_str = file_path
            .to_str()
            .unwrap_or_else(|| panic!("Unable to get file path for {:?}", &file_path));
        if file_path_str.ends_with(".yaml") {
            let kube_kind = to_kube_kind(file_path_str);
            kube_resources.insert(
                file.as_ref()
                    .unwrap_or_else(|_| panic!("Unable to get file {:?}", &file))
                    .file_name()
                    .to_str()
                    .unwrap_or_else(|| panic!("Unable to get file name for {:?}", &file))
                    .to_string(),
                kube_kind,
            );
        }
    }

    kube_resources
}

#[cfg(feature = "test-local-kube")]
#[test]
fn q_ingress_test() {
    let (context, event_details) = router_context();
    let chart_name = "q-ingress-tls";
    let chart = CommonChart {
        chart_info: ChartInfo {
            name: chart_name.to_string(),
            path: chart_path(chart_name),
            namespace: HelmChartNamespaces::KubeSystem,
            custom_namespace: None,
            action: HelmAction::Deploy,
            atomic: false,
            force_upgrade: false,
            recreate_pods: false,
            last_breaking_version_requiring_restart: None,
            timeout_in_seconds: 0,
            dry_run: false,
            wait: false,
            values: vec![],
            values_string: vec![],
            values_files: vec![],
            yaml_files_content: vec![],
            parse_stderr_for_error: false,
            k8s_selector: None,
            backup_resources: None,
            crds_update: None,
        },
        chart_installation_checker: None,
    };

    let resources = get_kube_resources(
        format!("{}/common/charts/{}", lib_dir(), chart_name).as_str(),
        chart.chart_info,
        None,
        context,
        event_details,
    );
    assert!(!resources.is_empty());

    let cert_issuer = resources.get("cert-issuer.yaml").unwrap();
    assert_eq!(cert_issuer.types.as_ref().unwrap().kind, "Issuer");
}

#[cfg(feature = "test-local-kube")]
#[test]
fn q_container_test() {
    let (context, event_details) = container_context();
    let chart_name = "q-container";
    let chart = CommonChart {
        chart_info: ChartInfo {
            name: chart_name.to_string(),
            path: chart_path(chart_name),
            namespace: HelmChartNamespaces::KubeSystem,
            custom_namespace: None,
            action: HelmAction::Deploy,
            atomic: false,
            force_upgrade: false,
            recreate_pods: false,
            last_breaking_version_requiring_restart: None,
            timeout_in_seconds: 0,
            dry_run: false,
            wait: false,
            values: vec![],
            values_string: vec![],
            values_files: vec![],
            yaml_files_content: vec![],
            parse_stderr_for_error: false,
            k8s_selector: None,
            backup_resources: None,
            crds_update: None,
        },
        chart_installation_checker: None,
    };
    let resources = get_kube_resources(
        format!("{}/common/charts/{}", lib_dir(), chart_name).as_str(),
        chart.chart_info,
        None,
        context,
        event_details,
    );
    assert!(!resources.is_empty());
}

#[cfg(feature = "test-local-kube")]
#[test]
fn q_application_test() {
    let (context, event_details) = application_context();
    let chart_name = "q-application";
    let chart = CommonChart {
        chart_info: ChartInfo {
            name: chart_name.to_string(),
            path: chart_path(chart_name),
            namespace: HelmChartNamespaces::KubeSystem,
            custom_namespace: None,
            action: HelmAction::Deploy,
            atomic: false,
            force_upgrade: false,
            recreate_pods: false,
            last_breaking_version_requiring_restart: None,
            timeout_in_seconds: 0,
            dry_run: false,
            wait: false,
            values: vec![],
            values_string: vec![],
            values_files: vec![],
            yaml_files_content: vec![],
            parse_stderr_for_error: false,
            k8s_selector: None,
            backup_resources: None,
            crds_update: None,
        },
        chart_installation_checker: None,
    };
    let resources = get_kube_resources(
        format!("{}/aws/charts/{}", lib_dir(), chart_name).as_str(),
        chart.chart_info,
        None,
        context,
        event_details,
    );
    assert!(!resources.is_empty());
}

#[cfg(feature = "test-local-kube")]
#[test]
fn q_container_psql_test() {
    let (context, event_details) = container_database_context();
    let chart_name = "postgresql";
    let chart = CommonChart {
        chart_info: ChartInfo {
            name: chart_name.to_string(),
            path: chart_path(chart_name),
            namespace: HelmChartNamespaces::KubeSystem,
            custom_namespace: None,
            action: HelmAction::Deploy,
            atomic: false,
            force_upgrade: false,
            recreate_pods: false,
            last_breaking_version_requiring_restart: None,
            timeout_in_seconds: 0,
            dry_run: false,
            wait: false,
            values: vec![],
            values_string: vec![],
            values_files: vec![],
            yaml_files_content: vec![],
            parse_stderr_for_error: false,
            k8s_selector: None,
            backup_resources: None,
            crds_update: None,
        },
        chart_installation_checker: None,
    };
    let resources = get_kube_resources(
        format!("{}/common/services/{}", lib_dir(), chart_name).as_str(),
        chart.chart_info,
        None,
        context,
        event_details,
    );
    assert!(!resources.is_empty());
}

#[cfg(feature = "test-local-kube")]
#[test]
fn q_managed_psql_test() {
    let (context, event_details) = managed_database_context();
    let chart_name = "external-name-svc";
    let chart = CommonChart {
        chart_info: ChartInfo {
            name: chart_name.to_string(),
            path: chart_path(chart_name),
            namespace: HelmChartNamespaces::KubeSystem,
            custom_namespace: None,
            action: HelmAction::Deploy,
            atomic: false,
            force_upgrade: false,
            recreate_pods: false,
            last_breaking_version_requiring_restart: None,
            timeout_in_seconds: 0,
            dry_run: false,
            wait: false,
            values: vec![],
            values_string: vec![],
            values_files: vec![],
            yaml_files_content: vec![],
            parse_stderr_for_error: false,
            k8s_selector: None,
            backup_resources: None,
            crds_update: None,
        },
        chart_installation_checker: None,
    };
    let resources = get_kube_resources(
        format!("{}/common/charts/{}", lib_dir(), chart_name).as_str(),
        chart.chart_info,
        None,
        context,
        event_details,
    );
    assert!(!resources.is_empty());
}

#[cfg(feature = "test-local-kube")]
#[test]
fn q_job_test() {
    let (context, event_details) = job_context();
    let chart_name = "q-job";
    let chart = CommonChart {
        chart_info: ChartInfo {
            name: chart_name.to_string(),
            path: chart_path(chart_name),
            namespace: HelmChartNamespaces::KubeSystem,
            custom_namespace: None,
            action: HelmAction::Deploy,
            atomic: false,
            force_upgrade: false,
            recreate_pods: false,
            last_breaking_version_requiring_restart: None,
            timeout_in_seconds: 0,
            dry_run: false,
            wait: false,
            values: vec![],
            values_string: vec![],
            values_files: vec![],
            yaml_files_content: vec![],
            parse_stderr_for_error: false,
            k8s_selector: None,
            backup_resources: None,
            crds_update: None,
        },
        chart_installation_checker: None,
    };
    let resources = get_kube_resources(
        format!("{}/common/charts/{}", lib_dir(), chart_name).as_str(),
        chart.chart_info,
        None,
        context,
        event_details,
    );
    assert!(!resources.is_empty());
}
