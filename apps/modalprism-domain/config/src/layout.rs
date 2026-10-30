//! 配置目录布局：顶层区域与生命周期的唯一事实源。
//!
//! 加载器判定「什么算声明」、出厂基线冻结前剔除什么、退役清理删除什么、元数据哪些
//! kind 随之作废，都从这里取事实。新增或下线一个顶层目录只改这张表，不要在调用方
//! 各写一遍前缀判断——那种散落正是历史上「已下线目录能一路追进出厂快照」的来源。

use crate::scope::DocKind;

/// 顶层区域在配置生命周期里的角色。
pub enum Role {
    /// 参与配置快照的声明目录（`system.yaml` 是根上的声明文件）。
    Declaration(DocKind),
    /// 由专门读取器处理，不进快照，也不属于退役范围。
    Subsystem,
    /// 功能已下线：不进快照，且应被退役清理移除。
    Retired(Retired),
}

/// 已下线区域的退役信息。
pub struct Retired {
    /// 下线标记，写进日志与审计，便于追溯是哪一代功能留下的。
    pub since: &'static str,
    /// 一并作废的元数据 kind：配置目录没了，这些文档也没有读取方。
    ///
    /// 只登记语义唯一的名字；`candidate`/`review`/`draft` 这类通用词留给明确的
    /// 清空域，避免将来新功能复用同名 kind 时被启动清理误删。
    pub kinds: &'static [&'static str],
}

pub struct Area {
    /// 顶层名：目录名，或根下的声明文件名。
    pub name: &'static str,
    pub purpose: &'static str,
    pub role: Role,
}

pub const AREAS: &[Area] = &[
    Area {
        name: "system.yaml",
        purpose: "系统声明（策略预设入口）",
        role: Role::Declaration(DocKind::System),
    },
    Area {
        name: "defaults",
        purpose: "各声明种类的默认值",
        role: Role::Declaration(DocKind::Defaults),
    },
    Area {
        name: "presets",
        purpose: "策略预设",
        role: Role::Declaration(DocKind::Preset),
    },
    Area {
        name: "profiles",
        purpose: "可复用的策略剖面",
        role: Role::Declaration(DocKind::Profile),
    },
    Area {
        name: "accessors",
        purpose: "访问器声明",
        role: Role::Declaration(DocKind::Accessor),
    },
    Area {
        name: "adapters",
        purpose: "适配器声明",
        role: Role::Declaration(DocKind::Adapter),
    },
    Area {
        name: "bindings",
        purpose: "信源与适配器的绑定装配",
        role: Role::Declaration(DocKind::AdapterBinding),
    },
    Area {
        name: "sources",
        purpose: "信源声明",
        role: Role::Declaration(DocKind::Source),
    },
    Area {
        name: "plans",
        purpose: "采集计划",
        role: Role::Declaration(DocKind::Plan),
    },
    Area {
        name: "types",
        purpose: "内容类型声明",
        role: Role::Declaration(DocKind::Type),
    },
    Area {
        name: "schemas",
        purpose: "内容 schema（由 read_schemas 单独读取）",
        role: Role::Subsystem,
    },
    Area {
        name: "models",
        purpose: "模型注册（由 settings 的模型种子读取）",
        role: Role::Subsystem,
    },
    Area {
        name: "workflows",
        purpose: "加工工作流（功能已下线，仅存历史文件）",
        role: Role::Retired(Retired {
            since: "加工工作流下线",
            kinds: &[
                "input_set",
                "workflow",
                "workflow_version",
                "production_run",
                "production_attempt",
            ],
        }),
    },
];

/// 顶层名对应的区域。
fn area(name: &str) -> Option<&'static Area> {
    AREAS.iter().find(|area| area.name == name)
}

/// 逻辑路径的上层名（首段）。
pub(crate) fn top_segment(logical: &str) -> &str {
    logical.split('/').next().unwrap_or(logical)
}

/// 声明类型；非声明路径返回说明。
pub(crate) fn kind_of(logical: &str) -> Result<DocKind, String> {
    if logical == "system.yaml" || logical == "system.yml" {
        return Ok(DocKind::System);
    }
    let Some(area) = area(top_segment(logical)) else {
        return Err(format!("未知的配置路径: {logical}"));
    };
    match area.role {
        Role::Declaration(kind) if logical.contains('/') => Ok(kind),
        Role::Declaration(_) => Err(format!("声明目录缺少文件名: {logical}")),
        Role::Subsystem => Err(format!("{} 由专门读取器处理: {logical}", area.purpose)),
        Role::Retired(_) => Err(format!("{logical} 属于已下线功能")),
    }
}

/// 整棵跳过：子系统或已下线区域。
pub(crate) fn is_ignored(logical: &str) -> bool {
    matches!(
        area(top_segment(logical)).map(|area| &area.role),
        Some(Role::Subsystem | Role::Retired(_))
    )
}

/// 参与快照的顶层声明路径（含根上的 `system.yaml`）。
pub fn declaration_paths() -> Vec<&'static str> {
    AREAS
        .iter()
        .filter(|area| matches!(area.role, Role::Declaration(_)))
        .map(|area| area.name)
        .collect()
}

/// 已下线的顶层路径，退役清理按此逐项移走。
pub fn retired_paths() -> Vec<&'static str> {
    AREAS
        .iter()
        .filter(|area| matches!(area.role, Role::Retired(_)))
        .map(|area| area.name)
        .collect()
}

/// 已下线功能遗留的元数据 kind（去重）。
pub fn retired_kinds() -> Vec<&'static str> {
    let mut kinds: Vec<&'static str> = Vec::new();
    for area in AREAS {
        let Role::Retired(retired) = &area.role else {
            continue;
        };
        for kind in retired.kinds {
            if !kinds.contains(kind) {
                kinds.push(kind);
            }
        }
    }
    kinds
}

/// 退役原因，写日志与审计用。
pub fn retired_since(dir: &str) -> Option<&'static str> {
    match &area(dir)?.role {
        Role::Retired(retired) => Some(retired.since),
        _ => None,
    }
}
