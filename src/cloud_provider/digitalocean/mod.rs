extern crate digitalocean;

use std::any::Any;

use digitalocean::DigitalOcean;
use uuid::Uuid;

use crate::cloud_provider::{kubernetes::Kind as KubernetesKind, CloudProvider, Kind, TerraformStateCredentials};
use crate::constants::DIGITAL_OCEAN_TOKEN;
use crate::errors::EngineError;
use crate::events::{EventDetails, InfrastructureStep, Stage, Transmitter};
use crate::io_models::context::Context;
use crate::io_models::QoveryIdentifier;
use crate::utilities::to_short_id;

pub mod do_api_common;
pub mod kubernetes;
pub mod models;
pub mod network;

pub struct DO {
    context: Context,
    id: String,
    long_id: Uuid,
    name: String,
    pub token: String,
    spaces_access_id: String,
    spaces_secret_key: String,
    region: String,
    terraform_state_credentials: TerraformStateCredentials,
}

impl DO {
    pub fn new(
        context: Context,
        long_id: Uuid,
        token: &str,
        spaces_access_id: &str,
        spaces_secret_key: &str,
        region: &str,
        name: &str,
        terraform_state_credentials: TerraformStateCredentials,
    ) -> Self {
        DO {
            context,
            id: to_short_id(&long_id),
            long_id,
            name: name.to_string(),
            token: token.to_string(),
            spaces_access_id: spaces_access_id.to_string(),
            spaces_secret_key: spaces_secret_key.to_string(),
            region: region.to_string(),
            terraform_state_credentials,
        }
    }

    pub fn client(&self) -> DigitalOcean {
        DigitalOcean::new(self.token.as_str()).unwrap()
    }
}

impl CloudProvider for DO {
    fn context(&self) -> &Context {
        &self.context
    }

    fn kind(&self) -> Kind {
        Kind::Do
    }

    fn kubernetes_kind(&self) -> KubernetesKind {
        KubernetesKind::Doks
    }

    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn organization_id(&self) -> &str {
        self.context.organization_short_id()
    }

    fn organization_long_id(&self) -> Uuid {
        *self.context.organization_long_id()
    }

    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn access_key_id(&self) -> String {
        self.spaces_access_id.to_string()
    }

    fn secret_access_key(&self) -> String {
        self.spaces_secret_key.to_string()
    }

    fn region(&self) -> String {
        self.region.to_string()
    }

    fn aws_sdk_client(&self) -> Option<aws_config::SdkConfig> {
        None
    }

    fn token(&self) -> &str {
        self.token.as_str()
    }

    fn is_valid(&self) -> Result<(), EngineError> {
        let event_details = self.get_event_details(Stage::Infrastructure(InfrastructureStep::RetrieveClusterConfig));
        let client = DigitalOcean::new(&self.token);
        match client {
            Ok(_x) => Ok(()),
            Err(_) => Err(EngineError::new_client_invalid_cloud_provider_credentials(event_details)),
        }
    }

    fn zones(&self) -> &Vec<String> {
        todo!()
    }

    fn credentials_environment_variables(&self) -> Vec<(&str, &str)> {
        vec![(DIGITAL_OCEAN_TOKEN, self.token.as_str())]
    }

    fn tera_context_environment_variables(&self) -> Vec<(&str, &str)> {
        vec![("digital_ocean_token", self.token.as_str())] // FIXME random key and value; is it good?
    }

    fn terraform_state_credentials(&self) -> &TerraformStateCredentials {
        &self.terraform_state_credentials
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn get_event_details(&self, stage: Stage) -> EventDetails {
        let context = self.context();
        EventDetails::new(
            None,
            QoveryIdentifier::new(*context.organization_long_id()),
            QoveryIdentifier::new(*context.cluster_long_id()),
            context.execution_id().to_string(),
            stage,
            self.to_transmitter(),
        )
    }

    fn to_transmitter(&self) -> Transmitter {
        Transmitter::CloudProvider(self.long_id, self.name.to_string())
    }
}
