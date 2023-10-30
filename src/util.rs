use std::{
    collections::HashMap,
    sync::atomic::{AtomicI64, Ordering},
};

use json::{object, JsonValue};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CmpValue {
    Int(i64),
    Str(String),
    Float(f64),
}

impl PartialEq<String> for CmpValue {
    fn eq(&self, other: &String) -> bool {
        match self {
            CmpValue::Int(i) => other.parse().ok() == Some(*i),
            CmpValue::Str(s) => s == other,
            CmpValue::Float(f) => other.parse().ok() == Some(*f),
        }
    }
}

impl PartialEq<String> for &CmpValue {
    fn eq(&self, other: &String) -> bool {
        match self {
            CmpValue::Int(i) => other.parse().ok() == Some(*i),
            CmpValue::Str(s) => s == other,
            CmpValue::Float(f) => other.parse().ok() == Some(*f),
        }
    }
}

impl PartialEq<Option<&String>> for CmpValue {
    fn eq(&self, other: &Option<&String>) -> bool {
        match other {
            &Some(other) => match self {
                CmpValue::Int(i) => other.parse().ok() == Some(*i),
                CmpValue::Str(s) => s == other,
                CmpValue::Float(f) => other.parse().ok() == Some(*f),
            },
            _ => false,
        }
    }
}

impl PartialEq<Option<&String>> for &CmpValue {
    fn eq(&self, other: &Option<&String>) -> bool {
        match other {
            &Some(other) => match self {
                CmpValue::Int(i) => other.parse().ok() == Some(*i),
                CmpValue::Str(s) => s == other,
                CmpValue::Float(f) => other.parse().ok() == Some(*f),
            },
            _ => false,
        }
    }
}

impl PartialEq<i64> for CmpValue {
    fn eq(&self, other: &i64) -> bool {
        match self {
            CmpValue::Int(i) => *i == *other,
            CmpValue::Str(_) => false,
            CmpValue::Float(_) => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash)]
pub enum HAAction {
    Scene(String),
    Service { ha_id: String, service: String },
}

static HAACTION_ID: AtomicI64 = AtomicI64::new(1024);

impl HAAction {
    pub fn as_json(&self) -> JsonValue {
        match self {
            HAAction::Scene(s) => object! {
              "type": "call_service",
              "domain": "scene",
              "service": "turn_on",
              "target": {
                "entity_id": s.clone()
              },

              "service_data": {},
              "id": HAACTION_ID.fetch_add(1, Ordering::Relaxed)
            },

            HAAction::Service { ha_id, service } => object! {
                "type": "call_service",
              "domain": "light",
              "service": service.clone(),
              "target": {
                "entity_id": ha_id.clone()
              },

              "service_data": {},
              "id": HAACTION_ID.fetch_add(1, Ordering::Relaxed)
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HAConnect {
    Text {
        line: u8,
        text: String,
        color: u16,
    },
    Button {
        button: u8,
        ha_id: String,
        cmp: CmpValue,
        text_on: String,
        text_off: String,
        action_on: HAAction,
        action_off: HAAction,
        color: u16,
    },
    Line {
        line: u8,
        ha_id: String,
        text: String,
        make_int: bool,
        color: u16,
    },
}

impl HAConnect {
    pub fn is_on(&self, state: &HashMap<String, String>) -> bool {
        match self {
            HAConnect::Button { ha_id, cmp, .. } => {
                let st = state.get(ha_id);
                cmp == st
            }
            _ => false,
        }
    }
}

impl HAConnect {
    pub fn ha_id<'d>(&'d self) -> &'d String {
        match self {
            HAConnect::Text { text, .. } => text,
            HAConnect::Button { ha_id, .. } => ha_id,
            HAConnect::Line { ha_id, .. } => ha_id,
        }
    }
}

pub fn traverse(json: &JsonValue, path: &[&str]) -> Option<String> {
    let mut thing = json;
    for item in path {
        match thing {
            JsonValue::Object(v) => match v.get(*item) {
                None => {
                    return None;
                }
                Some(v) => {
                    thing = v;
                }
            },
            _ => {
                return None;
            }
        }
    }

    match thing {
        JsonValue::String(s) => Some(s.clone()),
        JsonValue::Short(s) => Some(s.to_string()),
        _ => None,
    }
}
