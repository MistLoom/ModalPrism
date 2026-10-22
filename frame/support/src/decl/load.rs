//! 声明文件加载。

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::ability::AbilityDecl;
use super::package::AbilityPackageMeta;
use super::protocol::{CliInfo, GrpcInfo, McpInfo, ProtocolKind, SkillInfo, ToolsInfo};

fn read_yaml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("读取 {}: {e}", path.display()))?;
    serde_yaml::from_str(&raw).map_err(|e| format!("解析 {}: {e}", path.display()))
}

/// 加载 `ability.yml`。
pub fn load_ability_decl(path: &Path) -> Result<AbilityDecl, String> {
    let decl: AbilityDecl = read_yaml(path)?;
    decl.validate()?;
    Ok(decl)
}

/// 扫描能力根目录下已声明的协议目录（存在 `info.yml`）。
pub fn detect_protocols(root: &Path) -> BTreeSet<ProtocolKind> {
    let mut kinds = BTreeSet::new();
    for kind in [
        ProtocolKind::Cli,
        ProtocolKind::Http,
        ProtocolKind::Mcp,
        ProtocolKind::Fun,
        ProtocolKind::Grpc,
        ProtocolKind::Skill,
        ProtocolKind::Tools,
    ] {
        if !kind.requires_info_dir() {
            continue;
        }
        let info = root.join(kind.dir_name()).join("info.yml");
        if info.is_file() {
            kinds.insert(kind);
        }
    }
    let tools_info = root.join(ProtocolKind::Tools.dir_name()).join("info.yml");
    if tools_info.is_file() {
        kinds.insert(ProtocolKind::Tools);
    }
    kinds
}

pub fn load_cli_info(path: &Path) -> Result<CliInfo, String> {
    let info: CliInfo = read_yaml(path)?;
    super::protocol::validate_cli_info(&info)?;
    Ok(info)
}

pub fn load_mcp_info(path: &Path) -> Result<McpInfo, String> {
    let info: McpInfo = read_yaml(path)?;
    super::protocol::validate_mcp_info(&info)?;
    Ok(info)
}

pub fn load_grpc_info(path: &Path) -> Result<GrpcInfo, String> {
    let info: GrpcInfo = read_yaml(path)?;
    super::protocol::validate_grpc_info(&info)?;
    Ok(info)
}

pub fn load_skill_info(path: &Path) -> Result<SkillInfo, String> {
    let info: SkillInfo = read_yaml(path)?;
    super::protocol::validate_skill_info(&info)?;
    Ok(info)
}

pub fn load_tools_info(path: &Path) -> Result<ToolsInfo, String> {
    let info: ToolsInfo = read_yaml(path)?;
    super::protocol::validate_tools_info(&info)?;
    Ok(info)
}

/// 加载能力包完整元数据。
pub fn load_ability_package(root: &Path) -> Result<AbilityPackageMeta, String> {
    let yml = AbilityPackageMeta::ability_yml_path(root);
    let decl = load_ability_decl(&yml)?;
    let runtime_kind = super::runtime_resolve::resolve_runtime_kind(root, &decl.framework_version)?;
    let detected = detect_protocols(root);
    let declared: BTreeSet<ProtocolKind> = decl.protocols.iter().copied().collect();

    for kind in &decl.protocols {
        if kind.requires_info_dir() && !detected.contains(kind) {
            return Err(format!(
                "ability.yml 声明协议 {} 但缺少 {}/info.yml",
                kind.as_str(),
                kind.dir_name()
            ));
        }
    }
    for kind in &detected {
        if !declared.contains(kind) {
            return Err(format!(
                "存在 {}/info.yml 但 ability.yml protocols 未声明 {}",
                kind.dir_name(),
                kind.as_str()
            ));
        }
    }

    let cli = if declared.contains(&ProtocolKind::Cli) {
        Some(load_cli_info(&AbilityPackageMeta::protocol_info_path(
            root,
            ProtocolKind::Cli,
        ))?)
    } else {
        None
    };
    let mcp = if declared.contains(&ProtocolKind::Mcp) {
        Some(load_mcp_info(&AbilityPackageMeta::protocol_info_path(
            root,
            ProtocolKind::Mcp,
        ))?)
    } else {
        None
    };
    let grpc = if declared.contains(&ProtocolKind::Grpc) {
        Some(load_grpc_info(&AbilityPackageMeta::protocol_info_path(
            root,
            ProtocolKind::Grpc,
        ))?)
    } else {
        None
    };
    let skill = if declared.contains(&ProtocolKind::Skill) {
        Some(load_skill_info(&AbilityPackageMeta::protocol_info_path(
            root,
            ProtocolKind::Skill,
        ))?)
    } else {
        None
    };
    let tools = if declared.contains(&ProtocolKind::Tools) {
        let path = AbilityPackageMeta::protocol_info_path(root, ProtocolKind::Tools);
        if path.is_file() {
            Some(load_tools_info(&path)?)
        } else {
            None
        }
    } else {
        None
    };

    Ok(AbilityPackageMeta {
        root: root.to_path_buf(),
        decl,
        runtime_kind,
        protocols: declared,
        cli,
        mcp,
        grpc,
        skill,
        tools,
    })
}
