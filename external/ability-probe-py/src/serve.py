"""Guest 子进程固定入口 `src/serve.py`。"""

import sys
from pathlib import Path

for _anc in Path(__file__).resolve().parents:
    _sdk = _anc / "SDK" / "ability-sdk-py" / "src"
    if _sdk.is_dir():
        sys.path.insert(0, str(_sdk))
        break

from leaf_ability_sdk import serve_ability


def ping(ctx, _args):
    ctx.emit("pong", {"from": "probe"})
    return {"ok": True, "from": "external-py"}


if __name__ == "__main__":
    serve_ability({"ping": ping})
