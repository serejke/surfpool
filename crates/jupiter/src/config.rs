use serde::{Deserialize, Serialize};

/// Default base URL of Jupiter's keyless lite tier.
pub const DEFAULT_JUPITER_LITE_BASE_URL: &str = "https://lite-api.jup.ag/swap/v1";

/// Default base URL of Jupiter's paid pro tier.
pub const DEFAULT_JUPITER_PRO_BASE_URL: &str = "https://api.jup.ag/swap/v1";

/// Runtime configuration for the Jupiter extension.
///
/// Disabled by default — when [`JupiterConfig::enabled`] is `false`, surfpool
/// never registers the RPC methods and never reaches out to the upstream
/// Jupiter API.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JupiterConfig {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl Default for JupiterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: DEFAULT_JUPITER_LITE_BASE_URL.to_string(),
            api_key: None,
        }
    }
}
