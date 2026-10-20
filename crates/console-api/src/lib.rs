//! Web 控制台 REST API。仅面向环回部署。

mod error;
mod handlers;

use std::sync::Arc;

use assembly::Services;
use axum::Router;
use axum::routing::{get, patch, post};

pub use error::ApiError;

#[derive(Clone)]
pub struct ConsoleState {
    pub services: Arc<Services>,
    pub http_addr: String,
}

pub fn router(state: ConsoleState) -> Router {
    Router::new()
        .route("/console/api/v1/meta", get(handlers::meta))
        .route("/console/api/v1/overview", get(handlers::overview))
        .route("/console/api/v1/sources", get(handlers::sources_list))
        .route("/console/api/v1/sources/{id}", get(handlers::source_get))
        .route(
            "/console/api/v1/sources/{id}/trigger",
            post(handlers::source_trigger),
        )
        .route("/console/api/v1/sources/{id}", patch(handlers::source_patch))
        .route("/console/api/v1/records", get(handlers::records_list))
        .route("/console/api/v1/records/{id}", get(handlers::record_get))
        .route("/console/api/v1/governance/egress", get(handlers::governance_egress))
        .route(
            "/console/api/v1/governance/egress/{host}/reset-circuit",
            post(handlers::reset_circuit),
        )
        .route("/console/api/v1/config/summary", get(handlers::config_summary))
        .route("/console/api/v1/config/validate", post(handlers::config_validate))
        .route("/console/api/v1/config/reload", post(handlers::config_reload))
        .route(
            "/console/api/v1/config/files/{*path}",
            get(handlers::config_file_get).put(handlers::config_file_put),
        )
        .with_state(state)
}
