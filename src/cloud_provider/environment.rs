use crate::cloud_provider::service::Action;

use crate::events::{EnvironmentStep, EventDetails, Stage, Transmitter};
use crate::io_models::context::Context;
use crate::io_models::QoveryIdentifier;
use crate::models::application::ApplicationService;
use crate::models::container::ContainerService;
use crate::models::database::DatabaseService;
use crate::models::router::RouterService;
use crate::utilities::to_short_id;
use uuid::Uuid;

pub struct Environment {
    namespace: String,
    event_details: EventDetails,
    pub id: String,
    pub long_id: Uuid,
    pub project_id: String,
    pub project_long_id: Uuid,
    pub owner_id: String,
    pub organization_id: String,
    pub organization_long_id: Uuid,
    pub action: Action,
    pub applications: Vec<Box<dyn ApplicationService>>,
    pub containers: Vec<Box<dyn ContainerService>>,
    pub routers: Vec<Box<dyn RouterService>>,
    pub databases: Vec<Box<dyn DatabaseService>>,
}

impl Environment {
    pub fn new(
        long_id: Uuid,
        project_long_id: Uuid,
        organization_long_id: Uuid,
        action: Action,
        context: &Context,
        applications: Vec<Box<dyn ApplicationService>>,
        containers: Vec<Box<dyn ContainerService>>,
        routers: Vec<Box<dyn RouterService>>,
        databases: Vec<Box<dyn DatabaseService>>,
    ) -> Self {
        let project_id = to_short_id(&project_long_id);
        let env_id = to_short_id(&long_id);

        let stage = match action {
            Action::Create => Stage::Environment(EnvironmentStep::Deploy),
            Action::Pause => Stage::Environment(EnvironmentStep::Pause),
            Action::Delete => Stage::Environment(EnvironmentStep::Delete),
            Action::Nothing => Stage::Environment(EnvironmentStep::Deploy),
        };
        let event_details = EventDetails::new(
            None,
            QoveryIdentifier::new(*context.organization_long_id()),
            QoveryIdentifier::new(*context.cluster_long_id()),
            context.execution_id().to_string(),
            stage,
            Transmitter::Environment(long_id, "environment".to_string()),
        );

        Environment {
            event_details,
            namespace: format!("{}-{}", project_id, env_id),
            id: env_id,
            long_id,
            project_id,
            project_long_id,
            owner_id: "FAKE".to_string(),
            organization_id: to_short_id(&organization_long_id),
            organization_long_id,
            action,
            applications,
            containers,
            routers,
            databases,
        }
    }

    pub fn namespace(&self) -> &str {
        self.namespace.as_str()
    }

    pub fn event_details_with_step(&self, step: EnvironmentStep) -> EventDetails {
        EventDetails::clone_changing_stage(self.event_details.clone(), Stage::Environment(step))
    }

    pub fn event_details(&self) -> &EventDetails {
        &self.event_details
    }
}
