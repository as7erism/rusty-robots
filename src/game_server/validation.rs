use std::sync::Arc;

use ammonia::Builder;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Clone)]
#[error("{0}")]
pub struct ValidationError(pub Arc<str>);

// TODO check lengths

#[derive(Debug, Display, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Username(Arc<str>);

impl Username {
    pub fn validate(value: Arc<str>) -> Result<Self, ValidationError> {
        let cleaned = Builder::empty().clean(&value).to_string();
        if value.as_ref() == cleaned {
            Ok(Username(value))
        } else {
            Err(ValidationError("username contained html".into()))
        }
    }
}

#[derive(Debug, Display, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct Password(Arc<str>);

impl Password {
    pub fn validate(value: Arc<str>) -> Result<Self, ValidationError> {
        Ok(Password(value))
    }
}
