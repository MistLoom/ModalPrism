import sys
from pathlib import Path

for _anc in Path(__file__).resolve().parents:
    _sdk = _anc / "SDK" / "ability-sdk-py" / "src"
    if _sdk.is_dir():
        sys.path.insert(0, str(_sdk))
        break

from leaf_ability_sdk import serve_ability


class _Mw:
    mode = "middleware"

    def __init__(self, flags):
        self.flags = flags

    def around(self, ev, next_fn):
        out = next_fn(ev)
        self.flags["middleware"] = True
        return out


class _Pipe:
    mode = "pipeline"

    def __init__(self, flags):
        self.flags = flags

    def process(self, ev):
        self.flags["pipeline"] = True
        return ev


class _Sink:
    def __init__(self, flags, key):
        self.flags = flags
        self.mode = key
        self.key = key

    def on_event(self, _ev):
        self.flags[self.key] = True


def lanes(ctx, _args):
    flags = {
        "middleware": False,
        "pipeline": False,
        "parallel": False,
        "message": False,
    }
    filt = {"class": "ability", "name": "lane"}
    ctx.handle(filt, _Mw(flags))
    ctx.handle(filt, _Pipe(flags))
    ctx.handle(filt, _Sink(flags, "parallel"))
    ctx.handle(filt, _Sink(flags, "message"))
    ctx.emit("lane", {})
    for _ in range(40):
        if flags["message"]:
            break
        ctx.local_get("_drain")
    return flags


def do_filter(ctx, _args):
    filtered = ctx.with_protocol_filter("fun")
    filtered.local_set("pf", 1)
    v = ctx.local_get("pf")
    return {
        "shared_local": v == 1,
        "same_instance": filtered.instance == ctx.instance,
    }


def aspect(ctx, _args):
    ctx.register_aspect(None)
    return {}


if __name__ == "__main__":
    serve_ability({"lanes": lanes, "filter": do_filter, "aspect": aspect})
