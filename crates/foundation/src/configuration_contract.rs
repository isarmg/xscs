//! Build-time configuration materials derived from the runtime data type.
//!
//! The caller supplies its own current definition. No product source is read,
//! no process is started, and no installation state is created.

use schemars::JsonSchema;
use serde::Serialize;

mod rules;
pub use rules::validate_rules;

/// Generate the current JSON shape and defaults from one owned definition.
/// Product-specific semantic checks and path rules remain product authorities.
pub fn generate<T: JsonSchema + Serialize + Default>()
-> Result<serde_json::Value, serde_json::Error> {
    Ok(serde_json::json!({
        "schema": schemars::schema_for!(T),
        "defaults": serde_json::to_value(T::default())?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Default, Serialize, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct CurrentConfig {
        bind: String,
        limit: u64,
        #[serde(skip)]
        internal_identity: String,
    }

    #[test]
    fn schema_defaults_and_private_fields_use_the_same_serialized_definition() {
        assert!(CurrentConfig::default().internal_identity.is_empty());
        let material = generate::<CurrentConfig>().unwrap();
        let defaults = material["defaults"].as_object().unwrap();
        let properties = material["schema"]["properties"].as_object().unwrap();
        assert_eq!(
            defaults.keys().collect::<Vec<_>>(),
            properties.keys().collect::<Vec<_>>()
        );
        assert!(!properties.contains_key("internal_identity"));
        assert_eq!(material["schema"]["additionalProperties"], false);
        assert_eq!(properties["limit"]["minimum"], 0);
    }
}
