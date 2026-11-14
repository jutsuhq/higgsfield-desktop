//! The agent server as the app runs it (`AgentService.start`): listening from launch on the endpoint the helper
//! finds, and what Settings › Agents says of it.

use std::sync::{Arc, Mutex, PoisonError};

use jutsu_agent::{AgentEndpoint, AgentServer};

use super::service::{AgentService, AgentWindow};
use super::settings::AgentStatus;
use crate::AppEnvironment;

/// The handler, the server it answers for, and whether it listens.
pub struct AgentRuntime {
    pub service: Arc<AgentService>,
    server: Option<AgentServer>,
    status: Mutex<AgentStatus>,
}

impl std::fmt::Debug for AgentRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentRuntime")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl AgentRuntime {
    /// On `endpoint`, or where the helper looks (`AgentEndpoint::current`) when `None`.
    pub fn new(
        environment: Arc<AppEnvironment>,
        window: Arc<dyn AgentWindow>,
        app_version: &str,
        endpoint: Option<AgentEndpoint>,
    ) -> Self {
        let endpoint = endpoint.or_else(AgentEndpoint::current);
        let service = AgentService::new(environment, window, endpoint.as_ref(), app_version);
        let server = endpoint.map(|endpoint| AgentServer::new(endpoint, service.clone()));
        let status = if server.is_some() {
            AgentStatus::Idle
        } else {
            AgentStatus::NoEndpoint
        };
        Self {
            service,
            server,
            status: Mutex::new(status),
        }
    }

    /// Starts listening; call inside the tokio runtime. A failure is kept for Settings › Agents.
    pub fn start(&self) {
        let Some(server) = &self.server else {
            return;
        };
        let status = match server.start() {
            Ok(()) => {
                tracing::info!("listening for agents");
                let service = self.service.clone();
                tokio::spawn(async move { service.sweep_inbox().await });
                AgentStatus::Listening(server.endpoint.name.clone())
            }
            Err(error) => {
                tracing::error!(%error, "can't listen for agents");
                AgentStatus::Failed(error.to_string())
            }
        };
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = status;
    }

    pub fn stop(&self) {
        if let Some(server) = &self.server {
            server.stop();
        }
    }

    pub fn status(&self) -> AgentStatus {
        self.status.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}
