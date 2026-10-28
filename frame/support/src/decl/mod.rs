//! Ability 声明文件解析。运行时只读，与 codegen 共享 schema。

mod ability;
mod lint;
mod load;
mod manifest_build;
mod package;
mod protocol;
mod runtime_resolve;
mod version;

pub use ability::{AbilityDecl, MethodDecl, OutboundTopicDeclYaml, RuntimeKind};
pub use lint::{
    LintOptions, LintReport, assert_clean_reports, discover_ability_roots, lint_ability_package,
    lint_workspace,
};
pub use load::{
    detect_protocols, load_ability_decl, load_ability_package, load_cli_info, load_grpc_info,
    load_mcp_info, load_skill_info, load_tools_info,
};
pub use manifest_build::manifest_from_decl;
pub use package::AbilityPackageMeta;
pub use protocol::{
    CliArgDecl, CliInfo, GrpcInfo, GrpcRpcDecl, McpInfo, McpToolDecl, ProtocolInfo, ProtocolKind,
    SkillInfo, ToolsInfo, ToolsMethodDecl,
};
pub use runtime_resolve::parse_framework_tag;
pub use runtime_resolve::{
    GUEST_CJPM_TOML, GUEST_SERVE_CJ, GUEST_SERVE_MJS, GUEST_SERVE_PY, guest_entry_path,
    resolve_runtime_kind,
};
pub use version::{FRAMEWORK_VERSION, validate_framework_version, validate_schema_version};
