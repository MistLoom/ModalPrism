//! 能力包元数据（发现与注册）。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::ability::{AbilityDecl, RuntimeKind};
use super::manifest_build::manifest_from_decl;
use super::protocol::{
    CliInfo, GrpcInfo, McpInfo, ProtocolInfo, ProtocolKind, SkillInfo, ToolsInfo,
};
use crate::manifest::AbilityManifest;

/// 从磁盘解析的能力包元数据。
#[derive(Clone, Debug)]
pub struct AbilityPackageMeta {
    pub root: PathBuf,
    pub decl: AbilityDecl,
    pub runtime_kind: RuntimeKind,
    pub protocols: BTreeSet<ProtocolKind>,
    pub cli: Option<CliInfo>,
    pub mcp: Option<McpInfo>,
    pub grpc: Option<GrpcInfo>,
    pub skill: Option<SkillInfo>,
    pub tools: Option<ToolsInfo>,
}

impl AbilityPackageMeta {
    pub fn id(&self) -> &str {
        &self.decl.id
    }

    pub fn supports(&self, kind: ProtocolKind) -> bool {
        self.protocols.contains(&kind)
    }

    /// 已加载的协议 info 声明（按种类）。
    pub fn protocol_infos(&self) -> Vec<ProtocolInfo> {
        let mut out = Vec::new();
        if let Some(cli) = &self.cli {
            out.push(ProtocolInfo::Cli(cli.clone()));
        }
        if let Some(mcp) = &self.mcp {
            out.push(ProtocolInfo::Mcp(mcp.clone()));
        }
        if let Some(grpc) = &self.grpc {
            out.push(ProtocolInfo::Grpc(grpc.clone()));
        }
        if let Some(skill) = &self.skill {
            out.push(ProtocolInfo::Skill(skill.clone()));
        }
        if let Some(tools) = &self.tools {
            out.push(ProtocolInfo::Tools(tools.clone()));
        }
        out
    }

    pub fn ability_yml_path(root: &Path) -> PathBuf {
        root.join("ability.yml")
    }

    pub fn protocol_info_path(root: &Path, kind: ProtocolKind) -> PathBuf {
        root.join(kind.dir_name()).join("info.yml")
    }

    /// 将声明转为握手用 manifest。
    pub fn to_manifest(&self) -> Result<AbilityManifest, String> {
        manifest_from_decl(&self.decl)
    }
}
