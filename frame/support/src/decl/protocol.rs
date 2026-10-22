//! 协议声明类型。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::version::validate_framework_version;

/// 能力可声明支持的协议种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolKind {
    Cli,
    Http,
    Mcp,
    Fun,
    Grpc,
    Skill,
    /// LLM tool_calls 发现与调用边界（经 dispatch 等协议适配层）。
    Tools,
}

impl ProtocolKind {
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Http => "http",
            Self::Mcp => "mcp",
            Self::Fun => "fun",
            Self::Grpc => "grpc",
            Self::Skill => "skill",
            Self::Tools => "tools",
        }
    }

    /// 是否必须存在对应 `*/info.yml` 目录声明。
    pub fn requires_info_dir(self) -> bool {
        matches!(self, Self::Cli | Self::Mcp | Self::Grpc | Self::Skill)
    }

    pub fn from_dir(name: &str) -> Option<Self> {
        match name {
            "cli" => Some(Self::Cli),
            "http" => Some(Self::Http),
            "mcp" => Some(Self::Mcp),
            "fun" => Some(Self::Fun),
            "grpc" => Some(Self::Grpc),
            "skill" => Some(Self::Skill),
            "tools" => Some(Self::Tools),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        self.dir_name()
    }
}

/// CLI 参数声明。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CliArgDecl {
    pub name: String,
    #[serde(default)]
    pub positional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(default)]
    pub required: bool,
}

/// 协议 info 公共头（各协议 info 内联 `framework_version` 字段）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CliInfo {
    pub framework_version: String,
    pub binary: String,
    pub entry: String,
    #[serde(default)]
    pub args: Vec<CliArgDecl>,
}

/// MCP tool 投影覆盖。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpToolDecl {
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `mcp/info.yml`
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpInfo {
    pub framework_version: String,
    #[serde(default)]
    pub tools: Vec<McpToolDecl>,
}

/// gRPC RPC 映射。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GrpcRpcDecl {
    pub method: String,
    pub name: String,
}

/// `grpc/info.yml`
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GrpcInfo {
    pub framework_version: String,
    pub service: String,
    #[serde(default)]
    pub rpcs: Vec<GrpcRpcDecl>,
}

/// LLM tools 投影覆盖（`tools/info.yml`）。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolsMethodDecl {
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `tools/info.yml`
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolsInfo {
    pub framework_version: String,
    #[serde(default)]
    pub tools: Vec<ToolsMethodDecl>,
}

/// `skill/info.yml`
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillInfo {
    pub framework_version: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub disable_model_invocation: bool,
}

/// 协议 info 文件的通用包装。
#[derive(Clone, Debug)]
pub enum ProtocolInfo {
    Cli(CliInfo),
    Mcp(McpInfo),
    Grpc(GrpcInfo),
    Skill(SkillInfo),
    Tools(ToolsInfo),
}

impl ProtocolInfo {
    pub fn kind(&self) -> ProtocolKind {
        match self {
            Self::Cli(_) => ProtocolKind::Cli,
            Self::Mcp(_) => ProtocolKind::Mcp,
            Self::Grpc(_) => ProtocolKind::Grpc,
            Self::Skill(_) => ProtocolKind::Skill,
            Self::Tools(_) => ProtocolKind::Tools,
        }
    }
}

/// 将声明转为 JSON Schema 片段（供校验）。
pub fn validate_schema_fragment(schema: &Value) -> Result<(), String> {
    if schema.is_null() {
        return Ok(());
    }
    if !schema.is_object() {
        return Err("input schema 必须是 JSON object".into());
    }
    Ok(())
}

pub fn validate_cli_info(info: &CliInfo) -> Result<(), String> {
    validate_framework_version("cli/info.yml", Some(&info.framework_version))
}

pub fn validate_mcp_info(info: &McpInfo) -> Result<(), String> {
    validate_framework_version("mcp/info.yml", Some(&info.framework_version))
}

pub fn validate_grpc_info(info: &GrpcInfo) -> Result<(), String> {
    validate_framework_version("grpc/info.yml", Some(&info.framework_version))
}

pub fn validate_skill_info(info: &SkillInfo) -> Result<(), String> {
    validate_framework_version("skill/info.yml", Some(&info.framework_version))
}

pub fn validate_tools_info(info: &ToolsInfo) -> Result<(), String> {
    validate_framework_version("tools/info.yml", Some(&info.framework_version))
}
