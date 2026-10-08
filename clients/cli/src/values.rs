use crate::wire;
use serde_json::Value;

impl TryFrom<String> for wire::AgentSessionProviderDto {
    type Error = String;
    fn try_from(value: String) -> Result<Self, String> {
        Ok(Self {
            value: Some(match value.as_str() {
                "claude" => wire::agent_session_provider_dto::Value::Claude as i32,
                "codex" => wire::agent_session_provider_dto::Value::Codex as i32,
                _ => return Err(format!("Invalid AgentSessionProviderDto: {value}")),
            }),
        })
    }
}
impl TryFrom<Value> for wire::WorkflowValue {
    type Error = String;
    fn try_from(value: Value) -> Result<Self, String> {
        use wire::workflow_value::Variant;
        let variant = match value {
            Value::Null => Variant::NullValue(wire::Unit {}),
            Value::Bool(value) => Variant::BooleanValue(wire::ResultBool { value: Some(value) }),
            Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    Variant::SignedInteger(wire::WorkflowInteger { value: Some(value) })
                } else if let Some(value) = value.as_u64() {
                    Variant::UnsignedInteger(wire::ResultUint64 { value: Some(value) })
                } else {
                    Variant::NumberValue(wire::WorkflowNumber {
                        value: Some(value.as_f64().ok_or("Invalid workflow number")?),
                    })
                }
            }
            Value::String(value) => Variant::StringValue(wire::ResultString { value: Some(value) }),
            Value::Array(value) => Variant::ListValue(wire::WorkflowValueList {
                items: value
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_, _>>()?,
            }),
            Value::Object(value) => Variant::ObjectValue(wire::WorkflowValueObject {
                entries: value
                    .into_iter()
                    .map(|(key, value)| Ok((key, value.try_into()?)))
                    .collect::<Result<_, String>>()?,
            }),
        };
        Ok(Self {
            variant: Some(variant),
        })
    }
}
impl TryFrom<&str> for wire::AgentSessionProviderDto {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, String> {
        Self::try_from(value.to_owned())
    }
}
