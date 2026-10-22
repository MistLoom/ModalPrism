"""无第三方依赖的 ability.yml 子集解析。"""

from __future__ import annotations

import re
from pathlib import Path


def load_ability_yaml(root: Path) -> dict:
    text = (root / "ability.yml").read_text(encoding="utf-8")
    id_ = _scalar(text, "id")
    if not id_:
        raise RuntimeError("ability.yml 缺少 id")
    return {
        "id": id_,
        "framework_version": _scalar(text, "framework_version") or "1",
        "stateful": bool(re.search(r"^stateful:\s*true", text, re.M)),
        "methods": _methods(text),
        "events": _events(text),
        "depends": _depends(text),
    }


def _scalar(text: str, key: str) -> str | None:
    m = re.search(rf"^{key}:\s*[\"']?([^\"'\n#]+)[\"']?", text, re.M)
    return m.group(1).strip() if m else None


def _section(text: str, key: str) -> str | None:
    lines = text.splitlines()
    start = -1
    for i, line in enumerate(lines):
        if line.startswith(f"{key}:"):
            start = i + 1
            break
    if start < 0:
        return None
    body: list[str] = []
    for line in lines[start:]:
        if re.match(r"^[a-zA-Z_][\w.]*:", line):
            break
        body.append(line)
    return "\n".join(body)


def _methods(text: str) -> list[dict]:
    section = _section(text, "methods")
    if not section:
        return []
    return [
        {"id": mid, "description": "", "input": {"type": "object"}}
        for mid in re.findall(r"^\s+-\s+id:\s*(\S+)", section, re.M)
    ]


def _events(text: str) -> list[dict]:
    section = _section(text, "events")
    if not section:
        return []
    out = []
    for chunk in re.split(r"^\s+-\s+name:", section, flags=re.M)[1:]:
        name_m = re.match(r"\s*(\S+)", chunk)
        if not name_m:
            continue
        dir_m = re.search(r"dir:\s*(\S+)", chunk)
        out.append(
            {
                "name": name_m.group(1),
                "dir": dir_m.group(1) if dir_m else "emit",
                "description": "",
                "payload_schema": {"type": "object"},
            },
        )
    return out


def _depends(text: str) -> list[dict]:
    section = _section(text, "depends")
    if not section or re.match(r"^\s*\[\s*\]\s*$", section.strip()):
        return []
    out = []
    for chunk in re.split(r"^\s+-\s+", section, flags=re.M)[1:]:
        id_m = re.search(r"^\s*id:\s*(\S+)", chunk)
        if id_m:
            alias_m = re.search(r"alias:\s*(\S+)", chunk)
            out.append(
                {
                    "id": id_m.group(1),
                    "alias": alias_m.group(1) if alias_m else None,
                    "optional": False,
                },
            )
            continue
        simple = chunk.strip()
        if simple and ":" not in simple:
            out.append({"id": simple, "alias": None, "optional": False})
    return out


def schema_version(framework_version: str) -> str:
    if "@" in framework_version:
        return framework_version.split("@", 1)[1]
    return framework_version
