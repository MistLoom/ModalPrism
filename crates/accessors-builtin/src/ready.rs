//! 取消与截止时间。

use std::time::Instant;

use accessors::{AccessContext, AccessError};
use strata_model::ErrorClass;

pub fn ready(ctx: &AccessContext, deadline: Instant) -> Result<(), AccessError> {
    if ctx.is_cancelled() {
        return Err(AccessError::cancelled());
    }
    if Instant::now() >= deadline {
        return Err(AccessError::new(ErrorClass::Transient, "deadline"));
    }
    Ok(())
}

pub fn config(message: &str) -> AccessError {
    AccessError::new(ErrorClass::Config, message)
}

pub fn permanent(message: &str) -> AccessError {
    AccessError::new(ErrorClass::Permanent, message)
}

pub fn spec_error(source: impl std::error::Error + Send + Sync + 'static) -> AccessError {
    AccessError::with_source(ErrorClass::Config, "spec 无效", source)
}
