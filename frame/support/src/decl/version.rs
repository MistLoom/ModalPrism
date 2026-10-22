//! 声明文件框架版本。codegen 与运行时解析共用。

/// 当前声明 schema 版本。`ability.yml` 与各协议 `info.yml` 须与此一致。
pub const FRAMEWORK_VERSION: &str = "1";

/// 校验 schema 段（`framework_version` 在 `@` 之后的部分，或无前缀时整段）。
pub fn validate_schema_version(schema: &str) -> Result<(), String> {
    if schema == FRAMEWORK_VERSION {
        Ok(())
    } else {
        Err(format!(
            "framework_version schema 不兼容：声明 {schema}，当前 {FRAMEWORK_VERSION}"
        ))
    }
}

/// 校验声明中的 `framework_version`（支持 `node@1` 等运行时前缀）。
pub fn validate_framework_version(file: &str, declared: Option<&str>) -> Result<(), String> {
    match declared {
        None => Err(format!(
            "{file} 缺少 framework_version（当前要求 {FRAMEWORK_VERSION} 或 <runtime>@{FRAMEWORK_VERSION}）"
        )),
        Some(v) => {
            let (_, schema) = super::runtime_resolve::parse_framework_tag(v)?;
            validate_schema_version(schema).map_err(|e| format!("{file} {e}"))
        }
    }
}
