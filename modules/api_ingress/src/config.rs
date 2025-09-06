use serde::{Deserialize, Serialize};
/// API ingress configuration - reused from api_ingress module
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ApiIngressConfig {
    pub bind_addr: String,
    #[serde(default)]
    pub enable_docs: bool,
    #[serde(default)]
    pub cors_enabled: bool,
}
