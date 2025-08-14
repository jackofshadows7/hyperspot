use std::collections::BTreeMap;

use serde_json::Value;

use crate::model::ComponentsRegistry;

#[derive(serde::Serialize)]
pub struct OpenApi {
    pub openapi: &'static str,
    pub info: OpenApiInfo,
    pub paths: Value,
    pub components: Option<OpenApiComponents>,
}

#[derive(serde::Serialize)]
pub struct OpenApiInfo {
    pub title: &'static str,
    pub version: String,
    pub description: Option<&'static str>,
}

#[derive(serde::Serialize, Default)]
pub struct OpenApiComponents {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub schemas: BTreeMap<String, Value>,
}

impl OpenApiComponents {
    pub fn from_registry(registry: &ComponentsRegistry) -> Self {
        let schemas = registry
            .schemas
            .iter()
            .map(|(name, schema)| {
                (
                    name.clone(),
                    serde_json::to_value(schema).unwrap_or_default(),
                )
            })
            .collect();

        Self { schemas }
    }
}
