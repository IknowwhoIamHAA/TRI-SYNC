use std::collections::BTreeMap;

use serde_json::{Map, Value};

pub fn to_canonical_string(value: &Value) -> Result<String, serde_json::Error> {
    serde_json::to_string(&normalize(value))
}

fn normalize(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (key, value) in map {
                sorted.insert(key.clone(), normalize(value));
            }

            let mut normalized = Map::new();
            for (key, value) in sorted {
                normalized.insert(key, value);
            }
            Value::Object(normalized)
        }
        Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::to_canonical_string;

    #[test]
    fn encodes_objects_in_sorted_key_order() {
        let value = json!({
            "z": {"b": 2, "a": 1},
            "a": [ {"k": 2, "j": 1} ]
        });

        let canonical = to_canonical_string(&value).expect("canonical encoding should succeed");
        assert_eq!(canonical, r#"{"a":[{"j":1,"k":2}],"z":{"a":1,"b":2}}"#);
    }
}
