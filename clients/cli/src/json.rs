use prost::Message;
use prost_reflect::{DynamicMessage, FieldDescriptor, Kind, ReflectMessage, Value};
use serde_json::{Map, Value as Json};

fn flag(options: DynamicMessage, name: &str) -> bool {
    crate::descriptor::option(&options, name)
        .as_bool()
        .unwrap_or(false)
}
fn label(options: DynamicMessage, name: &str) -> String {
    crate::descriptor::option(&options, name)
        .as_str()
        .unwrap_or_default()
        .to_string()
}

pub fn from_message<M: Message>(name: &str, value: &M) -> Result<Json, String> {
    let descriptor = crate::descriptor::pool()
        .get_message_by_name(name)
        .ok_or_else(|| format!("Unknown message {name}"))?;
    let mut dynamic = DynamicMessage::new(descriptor);
    dynamic
        .transcode_from(value)
        .map_err(|error| error.to_string())?;
    from_dynamic(&dynamic)
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
