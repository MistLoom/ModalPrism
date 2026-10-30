"""Guest 子进程固定入口 `src/serve.py`。"""

import sys
from pathlib import Path

for _anc in Path(__file__).resolve().parents:
    _sdk = _anc / "SDK" / "ability-sdk-py" / "src"
    if _sdk.is_dir():
        sys.path.insert(0, str(_sdk))
        break

from leaf_ability_sdk import pending, serve_ability


def ping(ctx, _args):
    ctx.emit("pong", {"from": "probe"})
    return {"ok": True, "from": "external-py"}


def slow(_ctx, args):
    args = args or {}
    ms = int(args.get("ms", 150))
    fail = bool(args.get("fail"))

    def work():
        import time

        time.sleep(ms / 1000.0)
        if fail:
            raise RuntimeError("boom")
        return {"slept": ms}

    return pending(work)


if __name__ == "__main__":
    serve_ability({"ping": ping, "slow": slow})
