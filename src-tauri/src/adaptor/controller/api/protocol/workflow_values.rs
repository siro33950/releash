use super::client as wire;
use serde_json::Value;

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

impl TryFrom<wire::WorkflowValue> for Value {
    type Error = String;
    fn try_from(value: wire::WorkflowValue) -> Result<Self, String> {
        use wire::workflow_value::Variant;
        Ok(match value.variant.ok_or("Missing workflow value")? {
            Variant::NullValue(_) => Value::Null,
            Variant::BooleanValue(value) => value.value.ok_or("Missing boolean")?.into(),
            Variant::SignedInteger(value) => value.value.ok_or("Missing signed integer")?.into(),
            Variant::UnsignedInteger(value) => {
                value.value.ok_or("Missing unsigned integer")?.into()
            }
            Variant::NumberValue(value) => Value::Number(
                serde_json::Number::from_f64(value.value.ok_or("Missing number")?)
                    .ok_or("Nonfinite workflow number")?,
            ),
            Variant::StringValue(value) => value.value.ok_or("Missing string")?.into(),
            Variant::ListValue(value) => Value::Array(
                value
                    .items
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_, _>>()?,
            ),
            Variant::ObjectValue(value) => Value::Object(
                value
                    .entries
                    .into_iter()
                    .map(|(key, value)| Ok((key, value.try_into()?)))
                    .collect::<Result<_, String>>()?,
            ),
        })
    }
}

impl TryFrom<crate::usecase::workflow::WorkflowEventView> for wire::DurableWorkflowFactLogEntry {
    type Error = String;
    fn try_from(value: crate::usecase::workflow::WorkflowEventView) -> Result<Self, String> {
        Ok(Self {
            event: Some(value.event),
            execution_id: Some(value.execution_id),
            timestamp_ms: Some(value.timestamp_ms),
            payload: value
                .payload
                .into_iter()
                .map(|(key, value)| Ok((key, value.try_into()?)))
                .collect::<Result<_, String>>()?,
        })
    }
}
