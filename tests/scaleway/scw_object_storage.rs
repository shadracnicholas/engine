use crate::helpers::utilities::{context_for_resource, generate_id, FuncTestsSecrets};

use crate::helpers::scaleway::{SCW_RESOURCE_TTL_IN_SECONDS, SCW_TEST_ZONE};
use qovery_engine::object_storage::scaleway_object_storage::{BucketDeleteStrategy, ScalewayOS};
use qovery_engine::object_storage::ObjectStorage;
use tempfile::NamedTempFile;
use uuid::Uuid;

#[cfg(feature = "test-scw-infra")]
#[test]
fn test_delete_bucket_hard_delete_strategy() {
    // setup:
    let context = context_for_resource(Uuid::new_v4(), Uuid::new_v4());
    let secrets = FuncTestsSecrets::new();
    let scw_access_key = secrets.SCALEWAY_ACCESS_KEY.unwrap_or_else(|| "undefined".to_string());
    let scw_secret_key = secrets.SCALEWAY_SECRET_KEY.unwrap_or_else(|| "undefined".to_string());

    let scaleway_os = ScalewayOS::new(
        context,
        generate_id().to_string(),
        "test".to_string(),
        scw_access_key,
        scw_secret_key,
        SCW_TEST_ZONE,
        BucketDeleteStrategy::HardDelete,
        false,
        Some(SCW_RESOURCE_TTL_IN_SECONDS),
    );

    let bucket_name = format!("qovery-test-bucket-{}", generate_id());

    scaleway_os
        .create_bucket(bucket_name.as_str())
        .expect("error while creating object-storage bucket");

    // compute:
    let result = scaleway_os.delete_bucket(bucket_name.as_str());

    // validate:
    assert!(result.is_ok());
    assert!(!scaleway_os.bucket_exists(bucket_name.as_str()))
}

#[cfg(feature = "test-scw-infra")]
#[test]
fn test_delete_bucket_empty_strategy() {
    // setup:
    let context = context_for_resource(Uuid::new_v4(), Uuid::new_v4());

    let secrets = FuncTestsSecrets::new();
    let scw_access_key = secrets.SCALEWAY_ACCESS_KEY.unwrap_or_else(|| "undefined".to_string());
    let scw_secret_key = secrets.SCALEWAY_SECRET_KEY.unwrap_or_else(|| "undefined".to_string());

    let scaleway_os = ScalewayOS::new(
        context,
        generate_id().to_string(),
        "test".to_string(),
        scw_access_key,
        scw_secret_key,
        SCW_TEST_ZONE,
        BucketDeleteStrategy::Empty,
        false,
        Some(SCW_RESOURCE_TTL_IN_SECONDS),
    );

    let bucket_name = format!("qovery-test-bucket-{}", generate_id());

    scaleway_os
        .create_bucket(bucket_name.as_str())
        .expect("error while creating object-storage bucket");

    // compute:
    let result = scaleway_os.delete_bucket(bucket_name.as_str());

    // validate:
    assert!(result.is_ok());
    assert!(scaleway_os.bucket_exists(bucket_name.as_str()));

    // clean-up:
    scaleway_os
        .delete_bucket(bucket_name.as_str())
        .unwrap_or_else(|_| panic!("error deleting object storage bucket {}", bucket_name));
}

#[cfg(feature = "test-scw-infra")]
#[test]
fn test_create_bucket() {
    // setup:
    let context = context_for_resource(Uuid::new_v4(), Uuid::new_v4());
    let secrets = FuncTestsSecrets::new();
    let scw_access_key = secrets.SCALEWAY_ACCESS_KEY.unwrap_or_else(|| "undefined".to_string());
    let scw_secret_key = secrets.SCALEWAY_SECRET_KEY.unwrap_or_else(|| "undefined".to_string());

    let scaleway_os = ScalewayOS::new(
        context,
        generate_id().to_string(),
        "test".to_string(),
        scw_access_key,
        scw_secret_key,
        SCW_TEST_ZONE,
        BucketDeleteStrategy::HardDelete,
        false,
        Some(SCW_RESOURCE_TTL_IN_SECONDS),
    );

    let bucket_name = format!("qovery-test-bucket-{}", generate_id());

    // compute:
    let result = scaleway_os.create_bucket(bucket_name.as_str());

    // validate:
    assert!(result.is_ok());
    assert!(scaleway_os.bucket_exists(bucket_name.as_str()));

    // clean-up:
    scaleway_os
        .delete_bucket(bucket_name.as_str())
        .unwrap_or_else(|_| panic!("error deleting object storage bucket {}", bucket_name));
}

#[cfg(feature = "test-scw-infra")]
#[test]
fn test_recreate_bucket() {
    // setup:
    let context = context_for_resource(Uuid::new_v4(), Uuid::new_v4());
    let secrets = FuncTestsSecrets::new();
    let scw_access_key = secrets.SCALEWAY_ACCESS_KEY.unwrap_or_else(|| "undefined".to_string());
    let scw_secret_key = secrets.SCALEWAY_SECRET_KEY.unwrap_or_else(|| "undefined".to_string());

    let scaleway_os = ScalewayOS::new(
        context,
        generate_id().to_string(),
        "test".to_string(),
        scw_access_key,
        scw_secret_key,
        SCW_TEST_ZONE,
        BucketDeleteStrategy::HardDelete,
        false,
        Some(SCW_RESOURCE_TTL_IN_SECONDS),
    );

    let bucket_name = format!("qovery-test-bucket-{}", generate_id());

    // compute & validate:
    let create_result = scaleway_os.create_bucket(bucket_name.as_str());
    assert!(create_result.is_ok());
    assert!(scaleway_os.bucket_exists(bucket_name.as_str()));

    let delete_result = scaleway_os.delete_bucket(bucket_name.as_str());
    assert!(delete_result.is_ok());
    assert!(!scaleway_os.bucket_exists(bucket_name.as_str()));

    let recreate_result = scaleway_os.create_bucket(bucket_name.as_str());
    assert!(recreate_result.is_ok());
    assert!(scaleway_os.bucket_exists(bucket_name.as_str()));

    // clean-up:
    scaleway_os
        .delete_bucket(bucket_name.as_str())
        .unwrap_or_else(|_| panic!("error deleting object storage bucket {}", bucket_name));
}

#[cfg(feature = "test-scw-infra")]
#[test]
fn test_put_file() {
    // setup:
    let context = context_for_resource(Uuid::new_v4(), Uuid::new_v4());
    let secrets = FuncTestsSecrets::new();
    let scw_access_key = secrets.SCALEWAY_ACCESS_KEY.unwrap_or_else(|| "undefined".to_string());
    let scw_secret_key = secrets.SCALEWAY_SECRET_KEY.unwrap_or_else(|| "undefined".to_string());

    let scaleway_os = ScalewayOS::new(
        context,
        generate_id().to_string(),
        "test".to_string(),
        scw_access_key,
        scw_secret_key,
        SCW_TEST_ZONE,
        BucketDeleteStrategy::HardDelete,
        false,
        Some(SCW_RESOURCE_TTL_IN_SECONDS),
    );

    let bucket_name = format!("qovery-test-bucket-{}", generate_id());
    let object_key = format!("test-object-{}", generate_id());

    scaleway_os
        .create_bucket(bucket_name.as_str())
        .expect("error while creating object-storage bucket");

    let temp_file = NamedTempFile::new().expect("error while creating tempfile");

    // compute:
    let result = scaleway_os.put(
        bucket_name.as_str(),
        object_key.as_str(),
        temp_file.into_temp_path().to_str().unwrap(),
    );

    // validate:
    assert!(result.is_ok());
    assert!(scaleway_os
        .get(bucket_name.as_str(), object_key.as_str(), false)
        .is_ok());

    // clean-up:
    scaleway_os
        .delete_bucket(bucket_name.as_str())
        .unwrap_or_else(|_| panic!("error deleting object storage bucket {}", bucket_name));
}

#[cfg(feature = "test-scw-infra")]
#[test]
fn test_get_file() {
    // setup:
    let context = context_for_resource(Uuid::new_v4(), Uuid::new_v4());
    let secrets = FuncTestsSecrets::new();
    let scw_access_key = secrets.SCALEWAY_ACCESS_KEY.unwrap_or_else(|| "undefined".to_string());
    let scw_secret_key = secrets.SCALEWAY_SECRET_KEY.unwrap_or_else(|| "undefined".to_string());

    let scaleway_os = ScalewayOS::new(
        context,
        generate_id().to_string(),
        "test".to_string(),
        scw_access_key,
        scw_secret_key,
        SCW_TEST_ZONE,
        BucketDeleteStrategy::HardDelete,
        false,
        Some(SCW_RESOURCE_TTL_IN_SECONDS),
    );

    let bucket_name = format!("qovery-test-bucket-{}", generate_id());
    let object_key = format!("test-object-{}", generate_id());

    scaleway_os
        .create_bucket(bucket_name.as_str())
        .expect("error while creating object-storage bucket");

    let temp_file = NamedTempFile::new().expect("error while creating tempfile");
    let tempfile_path = temp_file.into_temp_path();
    let tempfile_path = tempfile_path.to_str().unwrap();

    scaleway_os
        .put(bucket_name.as_str(), object_key.as_str(), tempfile_path)
        .unwrap_or_else(|_| panic!("error while putting file {} into bucket {}", tempfile_path, bucket_name));

    // compute:
    let result = scaleway_os.get(bucket_name.as_str(), object_key.as_str(), false);

    // validate:
    assert!(result.is_ok());
    assert!(scaleway_os
        .get(bucket_name.as_str(), object_key.as_str(), false)
        .is_ok());

    // clean-up:
    scaleway_os
        .delete_bucket(bucket_name.as_str())
        .unwrap_or_else(|_| panic!("error deleting object storage bucket {}", bucket_name));
}

#[cfg(feature = "test-scw-infra")]
#[test]
fn test_ensure_file_is_absent() {
    // setup:
    let context = context_for_resource(Uuid::new_v4(), Uuid::new_v4());
    let secrets = FuncTestsSecrets::new();
    let scw_access_key = secrets.SCALEWAY_ACCESS_KEY.unwrap_or_else(|| "undefined".to_string());
    let scw_secret_key = secrets.SCALEWAY_SECRET_KEY.unwrap_or_else(|| "undefined".to_string());

    let scaleway_os = ScalewayOS::new(
        context,
        generate_id().to_string(),
        "test".to_string(),
        scw_access_key,
        scw_secret_key,
        SCW_TEST_ZONE,
        BucketDeleteStrategy::HardDelete,
        false,
        Some(SCW_RESOURCE_TTL_IN_SECONDS),
    );

    let bucket_name = format!("qovery-test-bucket-{}", generate_id());
    let object_key = format!("test-object-{}", generate_id());

    scaleway_os
        .create_bucket(bucket_name.as_str())
        .expect("error while creating object-storage bucket");

    assert!(scaleway_os.ensure_file_is_absent(&bucket_name, &object_key).is_ok());

    let temp_file = NamedTempFile::new().expect("error while creating tempfile");
    let tempfile_path = temp_file.into_temp_path();
    let tempfile_path = tempfile_path.to_str().unwrap();

    scaleway_os
        .put(bucket_name.as_str(), object_key.as_str(), tempfile_path)
        .unwrap_or_else(|_| panic!("error while putting file {} into bucket {}", tempfile_path, bucket_name));

    assert!(scaleway_os.ensure_file_is_absent(&bucket_name, &object_key).is_ok());

    // clean-up:
    scaleway_os
        .delete_bucket(bucket_name.as_str())
        .unwrap_or_else(|_| panic!("error deleting object storage bucket {}", bucket_name));
}
