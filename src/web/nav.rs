use crate::web::state::AppState;

/// Shared nav bar context — bundles the feature-detection booleans that every
/// full-page template needs for rendering the navigation sidebar.
pub struct NavContext {
    pub fika_installed: bool,
    pub modsync_installed: bool,
    pub modsync_enabled: bool,
    pub svm_installed: bool,
}

impl NavContext {
    /// Build a `NavContext` from the current `AppState`.
    pub fn from_state(state: &AppState) -> Self {
        let modsync_enabled = state.config().modsync.as_ref().is_some_and(|ms| ms.enabled);
        Self {
            fika_installed: state.fika_installed,
            modsync_installed: state.is_modsync_installed(),
            modsync_enabled,
            svm_installed: state.is_svm_installed(),
        }
    }
}
