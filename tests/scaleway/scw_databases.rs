use crate::helpers::utilities::{
    context_for_resource, engine_run_test, generate_id, generate_password, get_pods, get_svc_name, init,
    is_pod_restarted_env, logger, FuncTestsSecrets,
};
use ::function_name::named;
use qovery_engine::cloud_provider::{Kind as ProviderKind, Kind};
use qovery_engine::io_models::database::{Database, DatabaseKind, DatabaseMode};
use qovery_engine::transaction::TransactionResult;
use tracing::{span, warn, Level};
use uuid::Uuid;

use crate::helpers;
use crate::helpers::common::ClusterDomain;
use crate::helpers::common::Infrastructure;
use crate::helpers::database::{database_test_environment, test_db};
use crate::helpers::scaleway::{
    clean_environments, scw_default_infra_config, SCW_MANAGED_DATABASE_DISK_TYPE, SCW_MANAGED_DATABASE_INSTANCE_TYPE,
    SCW_SELF_HOSTED_DATABASE_DISK_TYPE, SCW_SELF_HOSTED_DATABASE_INSTANCE_TYPE, SCW_TEST_ZONE,
};
use qovery_engine::cloud_provider::kubernetes::Kind as KubernetesKind;
use qovery_engine::io_models::application::{Port, Protocol};
use qovery_engine::io_models::context::CloneForTest;
use qovery_engine::io_models::database::DatabaseMode::{CONTAINER, MANAGED};
use qovery_engine::io_models::Action;
use qovery_engine::utilities::to_short_id;

/**
**
** Global database tests
**
**/

// to check overload between several databases and apps
#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn deploy_an_environment_with_3_databases_and_3_apps() {
    let test_name = function_name!();
    engine_run_test(|| {
        init();

        let span = span!(Level::INFO, "test", name = test_name);
        let _enter = span.enter();

        let logger = logger();
        let secrets = FuncTestsSecrets::new();
        let cluster_id = secrets
            .SCALEWAY_TEST_CLUSTER_LONG_ID
            .expect("SCALEWAY_TEST_CLUSTER_LONG_ID");
        let context = context_for_resource(
            secrets
                .SCALEWAY_TEST_ORGANIZATION_LONG_ID
                .expect("SCALEWAY_TEST_ORGANIZATION_LONG_ID"),
            cluster_id,
        );
        let infra_ctx = scw_default_infra_config(&context, logger.clone());
        let context_for_deletion = context.clone_not_same_execution_id();
        let infra_ctx_for_deletion = scw_default_infra_config(&context_for_deletion, logger.clone());
        let environment = helpers::database::environment_3_apps_3_databases(
            &context,
            SCW_SELF_HOSTED_DATABASE_INSTANCE_TYPE,
            SCW_SELF_HOSTED_DATABASE_DISK_TYPE,
            Kind::Scw,
        );

        let mut environment_delete = environment.clone();
        environment_delete.action = Action::Delete;
        let env_action = environment.clone();
        let env_action_delete = environment_delete.clone();

        let result = environment.deploy_environment(&env_action, &infra_ctx);
        assert!(matches!(result, TransactionResult::Ok));

        let result = environment_delete.delete_environment(&env_action_delete, &infra_ctx_for_deletion);
        assert!(matches!(result, TransactionResult::Ok));

        // delete images created during test from registries
        if let Err(e) = clean_environments(&context, vec![environment], secrets, SCW_TEST_ZONE) {
            warn!("cannot clean environments, error: {:?}", e);
        }

        test_name.to_string()
    })
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn deploy_an_environment_with_db_and_pause_it() {
    let test_name = function_name!();
    engine_run_test(|| {
        init();

        let span = span!(Level::INFO, "test", name = test_name);
        let _enter = span.enter();

        let logger = logger();
        let secrets = FuncTestsSecrets::new();
        let cluster_id = secrets
            .SCALEWAY_TEST_CLUSTER_LONG_ID
            .expect("SCALEWAY_TEST_CLUSTER_LONG_ID");
        let context = context_for_resource(
            secrets
                .SCALEWAY_TEST_ORGANIZATION_LONG_ID
                .expect("SCALEWAY_TEST_ORGANIZATION_LONG_ID"),
            cluster_id,
        );
        let infra_ctx = scw_default_infra_config(&context, logger.clone());
        let context_for_deletion = context.clone_not_same_execution_id();
        let infra_ctx_for_deletion = scw_default_infra_config(&context_for_deletion, logger.clone());
        let environment = helpers::environment::environment_2_app_2_routers_1_psql(
            &context,
            secrets
                .clone()
                .DEFAULT_TEST_DOMAIN
                .expect("DEFAULT_TEST_DOMAIN is not set in secrets")
                .as_str(),
            SCW_SELF_HOSTED_DATABASE_INSTANCE_TYPE,
            SCW_SELF_HOSTED_DATABASE_DISK_TYPE,
            Kind::Scw,
        );

        let mut environment_delete = environment.clone();
        environment_delete.action = Action::Delete;
        let env_action = environment.clone();
        let env_action_delete = environment_delete.clone();

        let result = environment.deploy_environment(&env_action, &infra_ctx);
        assert!(matches!(result, TransactionResult::Ok));

        let result = environment.pause_environment(&env_action, &infra_ctx);
        assert!(matches!(result, TransactionResult::Ok));

        // Check that we have actually 0 pods running for this db
        let app_name = format!("postgresql{}-0", environment.databases[0].name);
        let ret = get_pods(
            context.clone(),
            ProviderKind::Scw,
            environment.clone(),
            app_name.as_str(),
            secrets.clone(),
        );
        assert!(ret.is_ok());
        assert!(ret.unwrap().items.is_empty());

        let result = environment_delete.delete_environment(&env_action_delete, &infra_ctx_for_deletion);
        assert!(matches!(result, TransactionResult::Ok));

        // delete images created during test from registries
        if let Err(e) = clean_environments(&context, vec![environment], secrets, SCW_TEST_ZONE) {
            warn!("cannot clean environments, error: {:?}", e);
        }

        test_name.to_string()
    })
}

// Ensure a full environment can run correctly
#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn postgresql_deploy_a_working_development_environment_with_all_options() {
    let test_name = function_name!();
    engine_run_test(|| {
        init();

        let span = span!(Level::INFO, "test", name = test_name);
        let _enter = span.enter();

        let logger = logger();
        let secrets = FuncTestsSecrets::new();
        let cluster_id = secrets
            .SCALEWAY_TEST_CLUSTER_LONG_ID
            .expect("SCALEWAY_TEST_CLUSTER_LONG_ID");
        let context = context_for_resource(
            secrets
                .SCALEWAY_TEST_ORGANIZATION_LONG_ID
                .expect("SCALEWAY_TEST_ORGANIZATION_LONG_ID"),
            cluster_id,
        );

        let infra_ctx = scw_default_infra_config(&context, logger.clone());
        let context_for_deletion = context.clone_not_same_execution_id();
        let infra_ctx_for_deletion = scw_default_infra_config(&context_for_deletion, logger.clone());
        let test_domain = secrets
            .DEFAULT_TEST_DOMAIN
            .as_ref()
            .expect("DEFAULT_TEST_DOMAIN is not set in secrets");

        let environment = helpers::environment::environment_2_app_2_routers_1_psql(
            &context,
            test_domain.as_str(),
            SCW_SELF_HOSTED_DATABASE_INSTANCE_TYPE,
            SCW_SELF_HOSTED_DATABASE_DISK_TYPE,
            Kind::Scw,
        );
        let mut environment_delete = helpers::environment::environment_2_app_2_routers_1_psql(
            &context_for_deletion,
            test_domain.as_str(),
            SCW_SELF_HOSTED_DATABASE_INSTANCE_TYPE,
            SCW_SELF_HOSTED_DATABASE_DISK_TYPE,
            Kind::Scw,
        );

        environment_delete.action = Action::Delete;

        let env_action = environment.clone();
        let env_action_for_deletion = environment_delete.clone();

        let result = environment.deploy_environment(&env_action, &infra_ctx);
        assert!(matches!(result, TransactionResult::Ok));

        let result = environment_delete.delete_environment(&env_action_for_deletion, &infra_ctx_for_deletion);
        assert!(matches!(result, TransactionResult::Ok));

        // delete images created during test from registries
        if let Err(e) =
            clean_environments(&context, vec![environment, environment_delete], secrets.clone(), SCW_TEST_ZONE)
        {
            warn!("cannot clean environments, error: {:?}", e);
        }

        test_name.to_string()
    })
}

// Ensure redeploy works as expected
#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn postgresql_deploy_a_working_environment_and_redeploy() {
    use chrono::Utc;

    let test_name = function_name!();
    engine_run_test(|| {
        init();

        let span = span!(Level::INFO, "test", name = test_name);
        let _enter = span.enter();

        let logger = logger();
        let secrets = FuncTestsSecrets::new();
        let cluster_id = secrets
            .SCALEWAY_TEST_CLUSTER_LONG_ID
            .expect("SCALEWAY_TEST_CLUSTER_LONG_ID");
        let context = context_for_resource(
            secrets
                .SCALEWAY_TEST_ORGANIZATION_LONG_ID
                .expect("SCALEWAY_TEST_ORGANIZATION_LONG_ID"),
            cluster_id,
        );

        let infra_ctx = scw_default_infra_config(&context, logger.clone());
        let context_for_redeploy = context.clone_not_same_execution_id();
        let infra_ctx_for_redeploy = scw_default_infra_config(&context_for_redeploy, logger.clone());
        let context_for_delete = context.clone_not_same_execution_id();
        let infra_ctx_for_delete = scw_default_infra_config(&context_for_delete, logger.clone());

        let mut environment = helpers::environment::working_minimal_environment(&context);

        let app_name = format!("postgresql-app-{}", generate_id());
        let database_mode = CONTAINER;
        let database_host = get_svc_name(DatabaseKind::Postgresql, Kind::Scw).to_string();
        let database_port = 5432;
        let database_db_name = "postgres".to_string();
        let database_username = "superuser".to_string();
        let database_password = generate_password(CONTAINER);

        environment.databases = vec![Database {
            kind: DatabaseKind::Postgresql,
            action: Action::Create,
            long_id: Uuid::new_v4(),
            name: database_db_name.clone(),
            created_at: Utc::now(),
            version: "11.8.0".to_string(),
            fqdn_id: database_host.clone(),
            fqdn: database_host.clone(),
            port: database_port,
            username: database_username.clone(),
            password: database_password.clone(),
            total_cpus: "500m".to_string(),
            total_ram_in_mib: 512,
            disk_size_in_gib: 10,
            mode: database_mode.clone(),
            database_instance_type: if database_mode == MANAGED {
                SCW_MANAGED_DATABASE_INSTANCE_TYPE
            } else {
                SCW_SELF_HOSTED_DATABASE_INSTANCE_TYPE
            }
            .to_string(),
            database_disk_type: if database_mode == MANAGED {
                SCW_MANAGED_DATABASE_DISK_TYPE
            } else {
                SCW_SELF_HOSTED_DATABASE_DISK_TYPE
            }
            .to_string(),
            encrypt_disk: false,
            activate_high_availability: false,
            activate_backups: false,
            publicly_accessible: false,
        }];
        environment.applications = environment
            .applications
            .into_iter()
            .map(|mut app| {
                app.branch = app_name.clone();
                app.commit_id = "5990752647af11ef21c3d46a51abbde3da1ab351".to_string();
                app.ports = vec![Port {
                    id: "zdf7d6aad".to_string(),
                    long_id: Default::default(),
                    port: 1234,
                    name: None,
                    is_default: true,
                    publicly_accessible: true,
                    protocol: Protocol::HTTP,
                }];
                app.environment_vars = btreemap! {
                     "PG_DBNAME".to_string() => base64::encode(database_db_name.clone()),
                     "PG_HOST".to_string() => base64::encode(database_host.clone()),
                     "PG_PORT".to_string() => base64::encode(database_port.to_string()),
                     "PG_USERNAME".to_string() => base64::encode(database_username.clone()),
                     "PG_PASSWORD".to_string() => base64::encode(database_password.clone()),
                };
                app
            })
            .collect::<Vec<qovery_engine::io_models::application::Application>>();
        environment.routers[0].routes[0].service_long_id = environment.applications[0].long_id;

        let environment_to_redeploy = environment.clone();
        let environment_check = environment.clone();
        let env_action_redeploy = environment_to_redeploy.clone();

        let mut environment_delete = environment.clone();
        environment_delete.action = Action::Delete;
        let env_action = environment.clone();
        let env_action_delete = environment_delete.clone();

        let result = environment.deploy_environment(&env_action, &infra_ctx);
        assert!(matches!(result, TransactionResult::Ok));

        let result = environment_to_redeploy.deploy_environment(&env_action_redeploy, &infra_ctx_for_redeploy);
        assert!(matches!(result, TransactionResult::Ok));

        // TO CHECK: DATABASE SHOULDN'T BE RESTARTED AFTER A REDEPLOY
        let database_name = format!("postgresql-{}-0", to_short_id(&environment_check.databases[0].long_id));
        let (ret, _) = is_pod_restarted_env(
            context.clone(),
            ProviderKind::Scw,
            environment_check,
            database_name.as_str(),
            secrets.clone(),
        );
        assert!(ret);

        let result = environment_delete.delete_environment(&env_action_delete, &infra_ctx_for_delete);
        assert!(matches!(result, TransactionResult::Ok | TransactionResult::Error(_)));

        // delete images created during test from registries
        if let Err(e) = clean_environments(&context, vec![environment], secrets, SCW_TEST_ZONE) {
            warn!("cannot clean environments, error: {:?}", e);
        }

        test_name.to_string()
    })
}

/**
 **
 ** PostgreSQL tests
 **
 **/
#[allow(dead_code)]
fn test_postgresql_configuration(version: &str, test_name: &str, database_mode: DatabaseMode, is_public: bool) {
    let secrets = FuncTestsSecrets::new();
    let cluster_id = secrets
        .SCALEWAY_TEST_CLUSTER_LONG_ID
        .expect("SCALEWAY_TEST_CLUSTER_LONG_ID");
    let context = context_for_resource(
        secrets
            .SCALEWAY_TEST_ORGANIZATION_LONG_ID
            .expect("SCALEWAY_TEST_ORGANIZATION_LONG_ID"),
        cluster_id,
    );
    let environment = database_test_environment(&context);

    engine_run_test(|| {
        test_db(
            context,
            logger(),
            environment,
            secrets,
            version,
            test_name,
            DatabaseKind::Postgresql,
            KubernetesKind::ScwKapsule,
            database_mode,
            is_public,
            ClusterDomain::Default {
                cluster_id: to_short_id(&cluster_id),
            },
            None,
        )
    })
}

// Postgres self hosted environment
#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore = "No database deployed in this version"]
fn private_postgresql_v10_deploy_a_working_dev_environment() {
    test_postgresql_configuration("10", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore = "No database deployed in this version"]
fn public_postgresql_v10_deploy_a_working_dev_environment() {
    test_postgresql_configuration("10", function_name!(), CONTAINER, true);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn private_postgresql_v11_deploy_a_working_dev_environment() {
    test_postgresql_configuration("11", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore]
fn public_postgresql_v11_deploy_a_working_dev_environment() {
    test_postgresql_configuration("11", function_name!(), CONTAINER, true);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn private_postgresql_v12_deploy_a_working_dev_environment() {
    test_postgresql_configuration("12", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore]
fn public_postgresql_v12_deploy_a_working_dev_environment() {
    test_postgresql_configuration("12", function_name!(), CONTAINER, true);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn private_postgresql_v13_deploy_a_working_dev_environment() {
    test_postgresql_configuration("13", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore]
fn public_postgresql_v13_deploy_a_working_dev_environment() {
    test_postgresql_configuration("13", function_name!(), CONTAINER, true);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn private_postgresql_v14_deploy_a_working_dev_environment() {
    test_postgresql_configuration("14", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn public_postgresql_v14_deploy_a_working_dev_environment() {
    test_postgresql_configuration("14", function_name!(), CONTAINER, true);
}

// Postgres production environment
#[cfg(feature = "test-scw-managed-services")]
#[named]
#[test]
#[ignore = "No database deployed in this version"]
fn private_postgresql_v10_deploy_a_working_prod_environment() {
    test_postgresql_configuration("10", function_name!(), MANAGED, false);
}

#[cfg(feature = "test-scw-managed-services")]
#[named]
#[test]
#[ignore = "No database deployed in this version"]
fn public_postgresql_v10_deploy_a_working_prod_environment() {
    test_postgresql_configuration("10", function_name!(), MANAGED, true);
}

#[cfg(feature = "test-scw-managed-services")]
#[named]
#[test]
#[ignore]
fn private_postgresql_v11_deploy_a_working_prod_environment() {
    test_postgresql_configuration("11", function_name!(), MANAGED, false);
}

#[cfg(feature = "test-scw-managed-services")]
#[named]
#[test]
#[ignore]
fn public_postgresql_v11_deploy_a_working_prod_environment() {
    test_postgresql_configuration("11", function_name!(), MANAGED, true);
}

#[cfg(feature = "test-scw-managed-services")]
#[named]
#[test]
#[ignore]
fn private_postgresql_v12_deploy_a_working_prod_environment() {
    test_postgresql_configuration("12", function_name!(), MANAGED, false);
}

#[cfg(feature = "test-scw-managed-services")]
#[named]
#[test]
#[ignore]
fn public_postgresql_v12_deploy_a_working_prod_environment() {
    test_postgresql_configuration("12", function_name!(), MANAGED, true);
}

#[cfg(feature = "test-scw-managed-services")]
#[named]
#[test]
#[ignore]
fn private_postgresql_v13_deploy_a_working_prod_environment() {
    test_postgresql_configuration("13", function_name!(), MANAGED, false);
}

#[cfg(feature = "test-scw-managed-services")]
#[named]
#[test]
#[ignore]
fn public_postgresql_v13_deploy_a_working_prod_environment() {
    test_postgresql_configuration("13", function_name!(), MANAGED, true);
}

/**
 **
 ** MongoDB tests
 **
 **/
#[allow(dead_code)]
fn test_mongodb_configuration(version: &str, test_name: &str, database_mode: DatabaseMode, is_public: bool) {
    let secrets = FuncTestsSecrets::new();
    let cluster_id = secrets
        .SCALEWAY_TEST_CLUSTER_LONG_ID
        .expect("SCALEWAY_TEST_CLUSTER_LONG_ID");
    let context = context_for_resource(
        secrets
            .SCALEWAY_TEST_ORGANIZATION_LONG_ID
            .expect("SCALEWAY_TEST_ORGANIZATION_LONG_ID"),
        cluster_id,
    );
    let environment = database_test_environment(&context);

    engine_run_test(|| {
        test_db(
            context,
            logger(),
            environment,
            secrets,
            version,
            test_name,
            DatabaseKind::Mongodb,
            KubernetesKind::ScwKapsule,
            database_mode,
            is_public,
            ClusterDomain::Default {
                cluster_id: to_short_id(&cluster_id),
            },
            None,
        )
    })
}

// development environment
#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore = "No database deployed in this version"]
fn private_mongodb_v3_6_deploy_a_working_dev_environment() {
    test_mongodb_configuration("3.6", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore = "No database deployed in this version"]
fn public_mongodb_v3_6_deploy_a_working_dev_environment() {
    test_mongodb_configuration("3.6", function_name!(), CONTAINER, true);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn private_mongodb_v4_0_deploy_a_working_dev_environment() {
    test_mongodb_configuration("4.0", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore]
fn public_mongodb_v4_0_deploy_a_working_dev_environment() {
    test_mongodb_configuration("4.0", function_name!(), CONTAINER, true);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn private_mongodb_v4_2_deploy_a_working_dev_environment() {
    test_mongodb_configuration("4.2", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore]
fn public_mongodb_v4_2_deploy_a_working_dev_environment() {
    test_mongodb_configuration("4.2", function_name!(), CONTAINER, true);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn private_mongodb_v4_4_deploy_a_working_dev_environment() {
    test_mongodb_configuration("4.4", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn public_mongodb_v4_4_deploy_a_working_dev_environment() {
    test_mongodb_configuration("4.4", function_name!(), CONTAINER, true);
}

/**
 **
 ** MySQL tests
 **
 **/
#[allow(dead_code)]
fn test_mysql_configuration(version: &str, test_name: &str, database_mode: DatabaseMode, is_public: bool) {
    let secrets = FuncTestsSecrets::new();
    let cluster_id = secrets
        .SCALEWAY_TEST_CLUSTER_LONG_ID
        .expect("SCALEWAY_TEST_CLUSTER_LONG_ID");
    let context = context_for_resource(
        secrets
            .SCALEWAY_TEST_ORGANIZATION_LONG_ID
            .expect("SCALEWAY_TEST_ORGANIZATION_LONG_ID"),
        cluster_id,
    );
    let environment = database_test_environment(&context);

    engine_run_test(|| {
        test_db(
            context,
            logger(),
            environment,
            secrets,
            version,
            test_name,
            DatabaseKind::Mysql,
            KubernetesKind::ScwKapsule,
            database_mode,
            is_public,
            ClusterDomain::Default {
                cluster_id: to_short_id(&cluster_id),
            },
            None,
        )
    })
}

// MySQL self-hosted environment
#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore = "Only 1 deployed in this version"]
fn private_mysql_v5_7_deploy_a_working_dev_environment() {
    test_mysql_configuration("5.7", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore = "Only 1 deployed in this version"]
fn public_mysql_v5_7_deploy_a_working_dev_environment() {
    test_mysql_configuration("5.7", function_name!(), CONTAINER, true);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn private_mysql_v8_deploy_a_working_dev_environment() {
    test_mysql_configuration("8.0", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn public_mysql_v8_deploy_a_working_dev_environment() {
    test_mysql_configuration("8.0", function_name!(), CONTAINER, true);
}

// MySQL production environment (RDS)
#[cfg(feature = "test-scw-managed-services")]
#[named]
#[test]
#[ignore]
fn private_mysql_v8_deploy_a_working_prod_environment() {
    test_mysql_configuration("8.0", function_name!(), MANAGED, false);
}

#[cfg(feature = "test-scw-managed-services")]
#[named]
#[test]
#[ignore]
fn public_mysql_v8_deploy_a_working_prod_environment() {
    test_mysql_configuration("8.0", function_name!(), MANAGED, true);
}

/**
 **
 ** Redis tests
 **
 **/
#[allow(dead_code)]
fn test_redis_configuration(version: &str, test_name: &str, database_mode: DatabaseMode, is_public: bool) {
    let secrets = FuncTestsSecrets::new();
    let cluster_id = secrets
        .SCALEWAY_TEST_CLUSTER_LONG_ID
        .expect("SCALEWAY_TEST_CLUSTER_LONG_ID");
    let context = context_for_resource(
        secrets
            .SCALEWAY_TEST_ORGANIZATION_LONG_ID
            .expect("SCALEWAY_TEST_ORGANIZATION_LONG_ID"),
        cluster_id,
    );
    let environment = database_test_environment(&context);

    engine_run_test(|| {
        test_db(
            context,
            logger(),
            environment,
            secrets,
            version,
            test_name,
            DatabaseKind::Redis,
            KubernetesKind::ScwKapsule,
            database_mode,
            is_public,
            ClusterDomain::Default {
                cluster_id: to_short_id(&cluster_id),
            },
            None,
        )
    })
}

// Redis self-hosted environment
#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn private_redis_v5_deploy_a_working_dev_environment() {
    test_redis_configuration("5", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore]
fn public_redis_v5_deploy_a_working_dev_environment() {
    test_redis_configuration("5", function_name!(), CONTAINER, true);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn private_redis_v6_deploy_a_working_dev_environment() {
    test_redis_configuration("6", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
#[ignore]
fn public_redis_v6_deploy_a_working_dev_environment() {
    test_redis_configuration("6", function_name!(), CONTAINER, true);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn private_redis_v7_deploy_a_working_dev_environment() {
    test_redis_configuration("7", function_name!(), CONTAINER, false);
}

#[cfg(feature = "test-scw-self-hosted")]
#[named]
#[test]
fn public_redis_v7_deploy_a_working_dev_environment() {
    test_redis_configuration("7", function_name!(), CONTAINER, true);
}
