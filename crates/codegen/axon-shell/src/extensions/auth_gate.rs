use agent_client_protocol as acp;

use crate::auth::{AuthManager, AxonAuth};

/// Require Axon auth from a sync context, accepting tokens in the client-side buffer window.
pub(crate) fn require_axon_auth(
    auth_manager: &AuthManager,
    missing_message: &'static str,
    non_axon_message: &'static str,
) -> Result<AxonAuth, acp::Error> {
    let auth = auth_manager
        .current_or_expired()
        .ok_or_else(|| acp::Error::auth_required().data(missing_message))?;
    if !auth.is_axon_auth() {
        return Err(acp::Error::auth_required().data(non_axon_message));
    }
    Ok(auth)
}
