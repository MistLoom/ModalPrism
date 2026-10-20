//! 仓库自带的四套策略预设。

pub struct BuiltinPreset {
    pub id: &'static str,
    pub file_name: &'static str,
    pub source: &'static str,
}

pub fn builtin_presets() -> [BuiltinPreset; 4] {
    [
        BuiltinPreset {
            id: "preset/minimal",
            file_name: "minimal.yaml",
            source: include_str!("../../../config/presets/minimal.yaml"),
        },
        BuiltinPreset {
            id: "preset/standard",
            file_name: "standard.yaml",
            source: include_str!("../../../config/presets/standard.yaml"),
        },
        BuiltinPreset {
            id: "preset/archive",
            file_name: "archive.yaml",
            source: include_str!("../../../config/presets/archive.yaml"),
        },
        BuiltinPreset {
            id: "preset/debug",
            file_name: "debug.yaml",
            source: include_str!("../../../config/presets/debug.yaml"),
        },
    ]
}
