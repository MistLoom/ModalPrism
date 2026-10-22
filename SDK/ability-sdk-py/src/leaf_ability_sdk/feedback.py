"""Guest 反馈提交（对齐 `CtxOp::Feedback`）。"""

from __future__ import annotations

import time
import uuid


def submit_feedback(ctx, tags: list[str] | str, desc: str) -> None:
    tag_list = [tags] if isinstance(tags, str) else list(tags)
    ctx.ctx_op(
        {
            "op": "feedback",
            "record": {
                "id": str(uuid.uuid4()),
                "at": int(time.time() * 1000),
                "loc": {
                    "ability": getattr(ctx, "instance", None) or "guest",
                    "module": "serve",
                },
                "tags": [str(t) for t in tag_list],
                "desc": str(desc),
                "chain_id": getattr(ctx, "chain_id", None) or "detached",
                "span": getattr(ctx, "span", 0) or 0,
            },
        },
    )
