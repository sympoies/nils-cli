use std::fmt;

use serde::Deserialize;
use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value, from_slice};
use tokio_tungstenite::tungstenite::Message;

pub(in crate::codex_app_server) fn forwarded_server_value(
    message: &Message,
) -> Result<Option<Value>, String> {
    let (bytes, invalid) = match message {
        Message::Text(text) => (text.as_bytes(), "proxy observed malformed JSON text"),
        Message::Binary(bytes) => (bytes.as_ref(), "proxy observed malformed JSON binary data"),
        _ => return Ok(None),
    };
    from_slice::<ServerValue>(bytes)
        .map(|value| Some(value.0))
        .map_err(|_| invalid.to_string())
}

// These paths serve response authorization, bootstrap, and server_observation.
// Preserve explicit nulls and the presence of result, error, and method for
// response classification.
#[derive(Clone, Copy)]
enum Field {
    Object(&'static [(&'static str, Field)]),
    Keep,
    Discard,
}

use Field::{Discard, Keep, Object};

const TURN_ERROR: Field = Object(&[("codexErrorInfo", Keep)]);
const SERVER: Field = Object(&[
    ("id", Keep),
    ("method", Keep),
    ("error", Discard),
    (
        "result",
        Object(&[
            ("turn", Object(&[("id", Keep)])),
            (
                "thread",
                Object(&[("id", Keep), ("status", Object(&[("type", Keep)]))]),
            ),
            ("model", Keep),
            ("reasoning_effort", Keep),
            ("reasoningEffort", Keep),
            ("effort", Keep),
        ]),
    ),
    (
        "params",
        Object(&[
            ("threadId", Keep),
            ("turnId", Keep),
            ("requestId", Keep),
            ("isBlocking", Keep),
            ("mode", Keep),
            ("willRetry", Keep),
            ("error", TURN_ERROR),
            (
                "turn",
                Object(&[("id", Keep), ("status", Keep), ("error", TURN_ERROR)]),
            ),
            ("rateLimits", Keep),
            ("rateLimitsByLimitId", Keep),
        ]),
    ),
]);

struct ServerValue(Value);

impl<'de> Deserialize<'de> for ServerValue {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        SERVER.deserialize(de).map(Self)
    }
}

impl<'de> DeserializeSeed<'de> for Field {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
        match self {
            Keep => Value::deserialize(de),
            // Walk skipped values through serde_json's normal parser to keep
            // JSON validation and the nesting limit without building a tree.
            _ => de.deserialize_any(self),
        }
    }
}

impl<'de> Visitor<'de> for Field {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let fields = match self {
            Object(fields) => fields,
            _ => &[],
        };
        let mut selected = Map::new();
        while let Some(field) = map.next_key_seed(FieldKey(fields))? {
            match field {
                Some((key, field)) => {
                    selected.insert(key.to_string(), map.next_value_seed(field)?);
                }
                None => {
                    map.next_value_seed(Discard)?;
                }
            }
        }
        Ok(match self {
            Object(_) => Value::Object(selected),
            _ => Value::Null,
        })
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        while seq.next_element_seed(Discard)?.is_some() {}
        Ok(Value::Null)
    }

    fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
}

struct FieldKey(&'static [(&'static str, Field)]);

impl<'de> DeserializeSeed<'de> for FieldKey {
    type Value = Option<(&'static str, Field)>;

    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        de.deserialize_identifier(self)
    }
}

impl Visitor<'_> for FieldKey {
    type Value = Option<(&'static str, Field)>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object key")
    }

    fn visit_str<E: serde::de::Error>(self, key: &str) -> Result<Self::Value, E> {
        Ok(self.0.iter().copied().find(|(name, _)| *name == key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_app_server::{ServerProjection, json_rpc_response_id_key, server_observation};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn forwarded_response_observation_preserves_presence_and_last_value() {
        let input = r#"{"id":1,"i\u0064":"last","result":[{"plugins":[]}],"extra":true}"#;
        let response = forwarded_server_value(&Message::Text(input.into()))
            .unwrap()
            .unwrap();
        assert_eq!(response, json!({"id":"last","result":null}));
        assert_eq!(
            json_rpc_response_id_key(&response).as_deref(),
            Some(r#""last""#)
        );

        let input = br#"{"id":1,"method":null,"error":{"message":"unused"}}"#;
        let request = forwarded_server_value(&Message::Binary(input.as_slice().into()))
            .unwrap()
            .unwrap();
        assert_eq!(request, json!({"id":1,"method":null,"error":null}));
        assert_eq!(json_rpc_response_id_key(&request), None);
        let ServerProjection::Unique(error) = server_observation(&request) else {
            panic!("a response-shaped error must produce a unique observation");
        };
        assert_eq!(error, json!({"id":1,"error":{}}));
    }

    #[test]
    fn discarded_payload_still_requires_valid_json() {
        let input = r#"{"id":1,"result":{"plugins":[{"text":"\uD800"}]}}"#;
        assert_eq!(
            forwarded_server_value(&Message::Text(input.into())).unwrap_err(),
            "proxy observed malformed JSON text"
        );
        let input = b"{\"params\":{\"discarded\":\"\xff\"}}";
        assert_eq!(
            forwarded_server_value(&Message::Binary(input.as_slice().into())).unwrap_err(),
            "proxy observed malformed JSON binary data"
        );
    }
}
