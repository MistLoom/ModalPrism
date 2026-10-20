//! 出站错误。类别交给调度。

use strata_model::ErrorClass;

use accessors::AccessError;

pub(crate) fn permanent(message: &str) -> AccessError {
    AccessError::new(ErrorClass::Permanent, message)
}

pub(crate) fn transient(message: &str) -> AccessError {
    AccessError::new(ErrorClass::Transient, message)
}

pub(crate) fn store_error(source: impl std::error::Error + Send + Sync + 'static) -> AccessError {
    AccessError::with_source(ErrorClass::Transient, "控制热数据读写失败", source)
}
