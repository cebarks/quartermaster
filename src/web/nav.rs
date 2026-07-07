use crate::web::state::AppState;

/// Shared nav bar context — bundles the feature-detection booleans that every
/// full-page template needs for rendering the navigation sidebar.
pub struct NavContext {
    pub fika_installed: bool,
    pub convoy_enabled: bool,
    pub svm_installed: bool,
    #[allow(dead_code)] // ponytail: used in later tasks
    pub has_configs: bool,
}

impl NavContext {
    /// Build a `NavContext` from the current `AppState`.
    pub fn from_state(state: &AppState) -> Self {
        let convoy_enabled = state.config().convoy.as_ref().is_some_and(|c| c.enabled);
        let has_configs = state
            .config_mgmt
            .discover_configs(&state.db.lock())
            .map(|c| !c.is_empty())
            .unwrap_or(false);
        Self {
            fika_installed: state.fika_installed,
            convoy_enabled,
            svm_installed: state.is_svm_installed(),
            has_configs,
        }
    }
}
