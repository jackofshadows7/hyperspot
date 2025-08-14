use std::collections::BTreeMap;

use schemars::schema::RootSchema;
use serde_json::Value;

#[derive(Debug, Default, Clone)]
pub struct ComponentsRegistry {
    /// Schema name -> RootSchema (serialized to components.schemas)
    pub schemas: BTreeMap<String, RootSchema>,
}

impl ComponentsRegistry {
    /// Recursively rewrite all "$ref" values from Swagger2-style
    /// "#/definitions/*" to OpenAPI3-style "#/components/schemas/*".
    fn rewrite_refs_to_oas3(value: &mut Value) {
        match value {
            Value::Object(map) => {
                if let Some(Value::String(s)) = map.get_mut("$ref") {
                    const OLD: &str = "#/definitions/";
                    const NEW: &str = "#/components/schemas/";
                    if let Some(rest) = s.strip_prefix(OLD) {
                        *s = format!("{NEW}{rest}");
                    }
                }
                for v in map.values_mut() {
                    Self::rewrite_refs_to_oas3(v);
                }
            }
            Value::Array(arr) => {
                for v in arr {
                    Self::rewrite_refs_to_oas3(v);
                }
            }
            _ => {}
        }
    }

    /// Normalize a RootSchema for OAS3:
    /// - convert to JSON
    /// - rewrite $ref paths
    /// - remove top-level "definitions"
    /// - convert back to RootSchema
    fn normalize_for_oas3(schema: &RootSchema) -> Result<RootSchema, String> {
        let mut json = serde_json::to_value(schema).map_err(|e| format!("to_value failed: {e}"))?;

        Self::rewrite_refs_to_oas3(&mut json);

        if let Value::Object(ref mut map) = json {
            if map.remove("definitions").is_some() {
                tracing::debug!("Removed embedded 'definitions' from schema root");
            }
        }

        serde_json::from_value::<RootSchema>(json)
            .map_err(|e| format!("from_value failed after normalization: {e}"))
    }

    /// Register a schema component with conflict detection (normalized compare).
    /// Returns: true if inserted or replaced, false if identical (no-op).
    pub fn register_schema(&mut self, name: impl Into<String>, schema: RootSchema) -> bool {
        let name = name.into();

        // Normalize incoming schema first
        let normalized = match Self::normalize_for_oas3(&schema) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(schema_name = %name, %err, "Failed to normalize schema; inserting original as-is");
                // If normalization fails, try to insert original, but still detect conflicts.
                // We still want deterministic behavior, so compare JSON forms.
                match serde_json::to_value(&schema) {
                    Ok(new_json) => {
                        if let Some(existing) = self.schemas.get(&name) {
                            match serde_json::to_value(existing) {
                                Ok(existing_json) if existing_json == new_json => {
                                    tracing::debug!(schema_name = %name, "Identical schema re-registered, ignoring");
                                    return false; // no-op
                                }
                                Ok(_) | Err(_) => {
                                    tracing::warn!(schema_name = %name, "Schema conflict (non-normalized); overriding with original");
                                }
                            }
                        }
                        self.schemas.insert(name, schema);
                        return true;
                    }
                    Err(e) => {
                        tracing::error!(schema_name = %name, error = %e, "to_value failed; skipping schema registration");
                        return false;
                    }
                }
            }
        };

        // Normalized compare against possibly-normalized existing
        if let Some(existing) = self.schemas.get(&name) {
            // Compare normalized JSONs for equality
            let existing_json = match serde_json::to_value(existing) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(schema_name = %name, error = %e, "Existing schema to_value failed; will replace with normalized");
                    Value::Null
                }
            };
            let new_json = match serde_json::to_value(&normalized) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(schema_name = %name, error = %e, "New normalized schema to_value failed; aborting registration");
                    return false;
                }
            };

            if existing_json == new_json {
                tracing::debug!(schema_name = %name, "Identical schema re-registered, ignoring");
                return false; // no-op
            } else {
                tracing::warn!(schema_name = %name, "Schema conflict: overriding with new normalized schema");
            }
        }

        self.schemas.insert(name, normalized);
        true
    }

    #[allow(dead_code)]
    pub fn get_schema(&self, name: &str) -> Option<&RootSchema> {
        self.schemas.get(name)
    }

    #[allow(dead_code)]
    pub fn has_schema(&self, name: &str) -> bool {
        self.schemas.contains_key(name)
    }
}
