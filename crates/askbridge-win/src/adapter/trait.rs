use askbridge_core::{DispatchRequest, PreparationOutcome, PreparationPolicy, Result};

use super::PageSession;

/// Prepares a provider page without submitting the user's request.
pub trait ProviderAdapter: Send + Sync {
    /// Returns the provider identifier served by this adapter.
    fn id(&self) -> &str;

    /// Returns whether a page URL belongs to this provider.
    fn matches_url(&self, url: &str) -> bool;

    /// Focuses and prepares the page while preserving the manual-send boundary.
    fn prepare(
        &self,
        page: &mut PageSession<'_>,
        request: &DispatchRequest,
        policy: &PreparationPolicy,
    ) -> Result<PreparationOutcome>;
}
