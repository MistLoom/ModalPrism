//! 极简 YAML frontmatter 解析（仅支持扁平标量键值）。

use crate::error::SkillError;
use crate::skill::SkillMeta;

pub struct ParsedDocument {
    pub meta: SkillMeta,
    pub body: String,
}

pub fn parse_skill_md(raw: &str) -> Result<ParsedDocument, SkillError> {
    let raw = raw.trim_start_matches('\u{feff}');
    let Some(rest) = raw.strip_prefix("---") else {
        return Err(SkillError::InvalidMeta(
            "缺少 YAML frontmatter（需以 --- 开头）".into(),
        ));
    };
    let rest = rest.trim_start_matches(['\r', '\n']);
    let end = rest
        .find("\n---")
        .ok_or_else(|| SkillError::InvalidMeta("frontmatter 未闭合".into()))?;
    let yaml = &rest[..end];
    let body = rest[end + 4..].trim_start_matches(['\r', '\n']).to_string();

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut disable_model_invocation = false;

    for line in yaml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = unquote(value.trim());
        match key {
            "name" => name = Some(value),
            "description" => {
                // 支持 `>` 折叠块的首行简写：后续以缩进延续的行合并
                description = Some(value.trim_start_matches(['>', '|']).trim().to_string());
            }
            "disable-model-invocation" | "disable_model_invocation" => {
                disable_model_invocation = matches!(value.as_str(), "true" | "True" | "TRUE" | "1");
            }
            _ => {}
        }
    }

    // description 多行：扫描 frontmatter 中缩进续行（紧跟 description）
    if let Some(desc) = description.as_mut() {
        let mut collecting = false;
        let mut parts = Vec::new();
        for line in yaml.lines() {
            if line.trim_start().starts_with("description:") {
                collecting = true;
                continue;
            }
            if collecting {
                if line.starts_with(' ') || line.starts_with('\t') {
                    parts.push(line.trim().to_string());
                } else if line.trim().is_empty() {
                    continue;
                } else {
                    break;
                }
            }
        }
        if !parts.is_empty() {
            if desc.is_empty() {
                *desc = parts.join(" ");
            } else {
                desc.push(' ');
                desc.push_str(&parts.join(" "));
            }
        }
    }

    let name = name.ok_or_else(|| SkillError::InvalidMeta("缺少 name".into()))?;
    let description =
        description.ok_or_else(|| SkillError::InvalidMeta("缺少 description".into()))?;
    if name.is_empty() {
        return Err(SkillError::InvalidMeta("name 不能为空".into()));
    }

    Ok(ParsedDocument {
        meta: SkillMeta {
            name,
            description,
            disable_model_invocation,
        },
        body,
    })
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let raw = r#"---
name: demo
description: 演示 skill
disable-model-invocation: false
---
# Demo

body text
"#;
        let doc = parse_skill_md(raw).unwrap();
        assert_eq!(doc.meta.name, "demo");
        assert_eq!(doc.meta.description, "演示 skill");
        assert!(doc.body.contains("body text"));
    }
}
