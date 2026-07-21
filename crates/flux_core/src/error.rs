use thiserror::Error;

use crate::value::ValueType;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("unknown class '{0}'")]
    UnknownClass(String),
    #[error("class '{0}' cannot be created directly")]
    NotCreatable(String),
    #[error("instance not found")]
    InstanceNotFound,
    #[error("cannot modify the root or a service")]
    CannotModifyService,
    #[error("reparent would create a cycle")]
    WouldCreateCycle,
    #[error("unknown property '{0}'")]
    UnknownProperty(String),
    #[error("attributes cannot hold instance references")]
    AttributeNotData,
    #[error("attribute names must be non-empty")]
    BadAttributeName,
    #[error("type mismatch for '{prop}': expected {expected:?}, got {got:?}")]
    TypeMismatch {
        prop: String,
        expected: ValueType,
        got: ValueType,
    },
    #[error("scene load failed: {0}")]
    Load(String),
}
