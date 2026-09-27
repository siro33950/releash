use std::sync::LazyLock;

use prost::Message;
#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
use prost_reflect::MessageDescriptor;
use prost_reflect::{DescriptorPool, DynamicMessage, FieldDescriptor, Kind, ReflectMessage, Value};
use serde_json::{Map, Value as Json};

static POOL: LazyLock<DescriptorPool> = LazyLock::new(|| {
    DescriptorPool::decode(
        include_bytes!(concat!(env!("OUT_DIR"), "/client_descriptor.bin")).as_slice(),
    )
    .expect("client descriptors")
});

fn option(options: DynamicMessage, name: &str) -> Value {
    let extension = POOL
        .get_extension_by_name(&format!("releash.client.v1.{name}"))
        .expect("protocol option");
    options.get_extension(&extension).into_owned()
}
fn flag(options: DynamicMessage, name: &str) -> bool {
    option(options, name).as_bool().unwrap_or(false)
}
fn label(options: DynamicMessage, name: &str) -> String {
    option(options, name)
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
pub(super) fn to_message<M: Message + Default>(name: &str, value: Json) -> Result<M, String> {
    let descriptor = POOL
        .get_message_by_name(name)
        .ok_or_else(|| format!("Unknown message {name}"))?;
    to_dynamic(descriptor, value)?
        .transcode_to()
        .map_err(|error| error.to_string())
}

pub(crate) fn from_message<M: Message>(name: &str, value: &M) -> Result<Json, String> {
    let descriptor = POOL
        .get_message_by_name(name)
        .ok_or_else(|| format!("Unknown message {name}"))?;
    let mut dynamic = DynamicMessage::new(descriptor);
    dynamic
        .transcode_from(value)
        .map_err(|error| error.to_string())?;
    from_dynamic(&dynamic)
}

#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
fn to_dynamic(descriptor: MessageDescriptor, value: Json) -> Result<DynamicMessage, String> {
    let mut result = DynamicMessage::new(descriptor.clone());
    if flag(descriptor.options(), "json_unit") {
        return if value.is_null() {
            Ok(result)
        } else {
            Err("Expected null".into())
        };
    }
    let wrapper = label(descriptor.options(), "json_wrapper");
    let tag = label(descriptor.options(), "json_tag");
    let content = label(descriptor.options(), "json_content");
    if descriptor.oneofs().any(|oneof| oneof.name() == "variant") && wrapper.is_empty() {
        for field in descriptor.fields() {
            let input = if flag(descriptor.options(), "json_untagged") {
                Some(value.clone())
            } else if !tag.is_empty() && value[&tag].as_str() == Some(field.json_name()) {
                if content.is_empty() {
                    let mut fields = value.as_object().ok_or("Expected tagged object")?.clone();
                    fields.remove(&tag);
                    Some(
                        if matches!(field.kind(), Kind::Message(ref d) if flag(d.options(), "json_unit"))
                        {
                            Json::Null
                        } else {
                            Json::Object(fields)
                        },
                    )
                } else {
                    Some(value[&content].clone())
                }
            } else {
                None
            };
            if let Some(input) = input {
                match to_field(&field, input) {
                    Ok(value) => {
                        result.set_field(&field, value);
                        return Ok(result);
                    }
                    Err(_) if flag(descriptor.options(), "json_untagged") => {}
                    Err(error) => return Err(error),
                }
            }
        }
        return Err(format!("Invalid {} variant", descriptor.name()));
    }
    let mut fields = if !wrapper.is_empty() {
        Map::from_iter([(wrapper.clone(), value)])
    } else {
        value.as_object().ok_or("Expected object")?.clone()
    };
    for field in descriptor
        .fields()
        .filter(|field| !flag(field.options(), "json_flatten"))
        .chain(
            descriptor
                .fields()
                .filter(|field| flag(field.options(), "json_flatten")),
        )
    {
        let value = if flag(field.options(), "json_flatten") {
            Some(Json::Object(std::mem::take(&mut fields)))
        } else {
            fields
                .remove(field.json_name())
                .or_else(|| fields.remove(field.name()))
        };
        match value {
            Some(Json::Null) if flag(field.options(), "json_nullable") => {}
            Some(value) => {
                result.set_field(&field, to_field(&field, value)?);
            }
            None if flag(field.options(), "json_required") => {
                return Err(format!(
                    "Missing {}.{}",
                    descriptor.name(),
                    field.json_name()
                ))
            }
            None => {}
        }
    }
    if !fields.is_empty() {
        return Err(format!(
            "Unknown fields in {}: {:?}",
            descriptor.name(),
            fields.keys().collect::<Vec<_>>()
        ));
    }
    Ok(result)
}

#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
fn to_field(field: &FieldDescriptor, value: Json) -> Result<Value, String> {
    let literal = label(field.options(), "json_literal");
    if !literal.is_empty()
        && serde_json::from_str::<Json>(&literal).map_err(|error| error.to_string())? != value
    {
        return Err("Invalid literal".into());
    }

    if field.is_list() {
        return value
            .as_array()
            .ok_or("Expected list")?
            .iter()
            .map(|value| to_kind(field.kind(), value.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::List);
    }
    if field.is_map() {
        let Kind::Message(entry) = field.kind() else {
            unreachable!()
        };
        let kind = entry.get_field_by_name("value").unwrap().kind();
        return value
            .as_object()
            .ok_or("Expected map")?
            .iter()
            .map(|(key, value)| {
                Ok((
                    prost_reflect::MapKey::String(key.clone()),
                    to_kind(kind.clone(), value.clone())?,
                ))
            })
            .collect::<Result<_, String>>()
            .map(Value::Map);
    }
    to_kind(field.kind(), value)
}

#[cfg(any(test, all(debug_assertions, feature = "desktop")))]
fn to_kind(kind: Kind, value: Json) -> Result<Value, String> {
    Ok(match kind {
        Kind::Message(descriptor) => Value::Message(to_dynamic(descriptor, value)?),
        Kind::Enum(descriptor) => {
            let variant = descriptor
                .values()
                .find(|variant| {
                    let name = label(variant.options(), "json_enum_name");
                    !name.is_empty() && Some(name.as_str()) == value.as_str()
                })
                .ok_or("Invalid enum value")?;
            Value::EnumNumber(variant.number())
        }
        Kind::Bool => Value::Bool(value.as_bool().ok_or("Expected boolean")?),
        Kind::String => Value::String(value.as_str().ok_or("Expected string")?.to_string()),
        Kind::Uint32 | Kind::Fixed32 => Value::U32(
            value
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("Expected uint32")?,
        ),
        Kind::Uint64 | Kind::Fixed64 => Value::U64(value.as_u64().ok_or("Expected uint64")?),
        Kind::Int32 | Kind::Sint32 | Kind::Sfixed32 => Value::I32(
            value
                .as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or("Expected int32")?,
        ),
        Kind::Int64 | Kind::Sint64 | Kind::Sfixed64 => {
            Value::I64(value.as_i64().ok_or("Expected int64")?)
        }
        Kind::Double => Value::F64(
            value
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or("Expected finite number")?,
        ),
        Kind::Float => Value::F32(
            value
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or("Expected finite number")? as f32,
        ),
        Kind::Bytes => return Err("Unexpected bytes in command value".into()),
    })
}

fn from_dynamic(value: &DynamicMessage) -> Result<Json, String> {
    let descriptor = value.descriptor();
    if flag(descriptor.options(), "json_unit") {
        return Ok(Json::Null);
    }
    let wrapper = label(descriptor.options(), "json_wrapper");
    if descriptor.oneofs().any(|oneof| oneof.name() == "variant") && wrapper.is_empty() {
        let field = descriptor
            .fields()
            .find(|field| value.has_field(field))
            .ok_or("Missing variant")?;
        let item = from_kind(field.kind(), value.get_field(&field).as_ref())?;
        if flag(descriptor.options(), "json_untagged") {
            return Ok(item);
        }
        let tag = label(descriptor.options(), "json_tag");
        let content = label(descriptor.options(), "json_content");
        let mut fields = if content.is_empty() {
            item.as_object().cloned().unwrap_or_default()
        } else {
            Map::from_iter([(content, item)])
        };
        fields.insert(tag, Json::String(field.json_name().into()));
        return Ok(Json::Object(fields));
    }
    let mut fields = Map::new();
    for field in descriptor.fields() {
        let present = value.has_field(&field);
        if !present && flag(field.options(), "json_required") {
            return Err(format!(
                "Missing {}.{}",
                descriptor.name(),
                field.json_name()
            ));
        }
        let item = if !present
            && flag(field.options(), "json_nullable")
            && !flag(field.options(), "json_omit_none")
        {
            Json::Null
        } else if present || field.is_list() || field.is_map() {
            from_field(&field, value.get_field(&field).as_ref())?
        } else {
            continue;
        };
        let literal = label(field.options(), "json_literal");
        if !literal.is_empty()
            && serde_json::from_str::<Json>(&literal).map_err(|error| error.to_string())? != item
        {
            return Err("Invalid literal".into());
        }
        if flag(field.options(), "json_omit_empty")
            && (item.as_array().is_some_and(Vec::is_empty)
                || item.as_object().is_some_and(Map::is_empty))
        {
            continue;
        }
        if flag(field.options(), "json_flatten") {
            fields.extend(item.as_object().ok_or("Expected flattened object")?.clone());
        } else {
            fields.insert(field.json_name().into(), item);
        }
    }
    if !wrapper.is_empty() {
        return fields
            .remove(&wrapper)
            .ok_or("Missing wrapped value".into());
    }
    Ok(Json::Object(fields))
}

fn from_field(field: &FieldDescriptor, value: &Value) -> Result<Json, String> {
    match value {
        Value::List(items) => items
            .iter()
            .map(|item| from_kind(field.kind(), item))
            .collect::<Result<Vec<_>, _>>()
            .map(Json::Array),
        Value::Map(items) => {
            let Kind::Message(entry) = field.kind() else {
                unreachable!()
            };
            let kind = entry.get_field_by_name("value").unwrap().kind();
            items
                .iter()
                .map(|(key, value)| {
                    let prost_reflect::MapKey::String(key) = key else {
                        return Err("Expected string map key".into());
                    };
                    Ok((key.clone(), from_kind(kind.clone(), value)?))
                })
                .collect::<Result<Map<_, _>, _>>()
                .map(Json::Object)
        }
        _ => from_kind(field.kind(), value),
    }
}

fn from_kind(kind: Kind, value: &Value) -> Result<Json, String> {
    Ok(match value {
        Value::Message(value) => return from_dynamic(value),
        Value::EnumNumber(number) => {
            let Kind::Enum(descriptor) = kind else {
                return Err("Unexpected enum".into());
            };
            let variant = descriptor.get_value(*number).ok_or("Unknown enum value")?;
            let name = label(variant.options(), "json_enum_name");
            if name.is_empty() {
                return Err("Unknown enum value".into());
            }
            Json::String(name)
        }
        Value::Bool(value) => Json::Bool(*value),
        Value::String(value) => Json::String(value.clone()),
        Value::U32(value) => Json::from(*value),
        Value::U64(value) => Json::from(*value),
        Value::I32(value) => Json::from(*value),
        Value::I64(value) => Json::from(*value),
        Value::F32(value) if value.is_finite() => Json::from(*value),
        Value::F64(value) if value.is_finite() => Json::from(*value),
        _ => return Err("Invalid command value".into()),
    })
}
