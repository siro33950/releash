use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::domain::workflow::value_objects::PredicateError;
use crate::domain::workflow::Predicate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PredicateShapeError {
    InvalidPredicate,
    InvalidOperator,
    ExpectedArray,
    Empty,
}

impl std::fmt::Display for PredicateShapeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPredicate => "predicate must be a field reference or an and/or map",
            Self::InvalidOperator => "predicate map must contain exactly one key: and or or",
            Self::ExpectedArray => "predicate and/or must contain an array",
            Self::Empty => "predicate and/or must contain at least one element",
        })
    }
}

impl std::error::Error for PredicateShapeError {}

impl From<PredicateError> for PredicateShapeError {
    fn from(error: PredicateError) -> Self {
        match error {
            PredicateError::Empty => Self::Empty,
        }
    }
}

pub(super) fn parse_predicate(value: &Value) -> Result<Predicate<String>, PredicateShapeError> {
    match value {
        Value::String(reference) => Ok(Predicate::Ref(reference.clone())),
        Value::Object(map) => {
            if map.len() != 1 {
                return Err(PredicateShapeError::InvalidOperator);
            }
            let (operator, value) = map.iter().next().unwrap();
            if !matches!(operator.as_str(), "and" | "or") {
                return Err(PredicateShapeError::InvalidOperator);
            }
            let Value::Array(elements) = value else {
                return Err(PredicateShapeError::ExpectedArray);
            };
            let predicates = elements
                .iter()
                .map(parse_predicate)
                .collect::<Result<_, _>>()?;
            if operator == "and" {
                Predicate::and(predicates)
            } else {
                Predicate::or(predicates)
            }
            .map_err(PredicateShapeError::from)
        }
        _ => Err(PredicateShapeError::InvalidPredicate),
    }
}

impl<'de> Deserialize<'de> for Predicate<String> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        parse_predicate(&value).map_err(serde::de::Error::custom)
    }
}

impl Serialize for Predicate<String> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (operator, predicates) = match self {
            Self::Ref(reference) => return reference.serialize(serializer),
            Self::And(predicates) => ("and", predicates),
            Self::Or(predicates) => ("or", predicates),
        };
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(operator, predicates)?;
        map.end()
    }
}

#[cfg(test)]
#[path = "predicate_wire_test.rs"]
mod predicate_wire_tests;
