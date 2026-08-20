mod attachment;
mod composer;
mod generic;
mod javascript;
mod login;
mod provider_health;
mod rules;
mod rules_update;
mod session;
mod temp_image;
mod r#trait;

pub use generic::GenericProviderAdapter;
pub use provider_health::{
    ProviderHealth, ProviderHealthCheck, ProviderHealthReport, check_provider_health,
};
pub use session::PageSession;
pub(crate) use temp_image::cleanup_stale_temp_images;
pub use r#trait::ProviderAdapter;

pub(crate) fn validate_builtin_rules() -> askbridge_core::Result<()> {
    rules::validate_builtin_rules()
}

pub(crate) use rules_update::refresh_rules_from_environment;
