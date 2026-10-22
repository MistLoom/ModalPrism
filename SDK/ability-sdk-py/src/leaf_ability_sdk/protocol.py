import json
import uuid
from typing import Any

PROTOCOL_VERSION = "1"


def encode_frame(frame: dict) -> str:
    return json.dumps(frame, ensure_ascii=False, separators=(",", ":")) + "\n"


def decode_frame(line: str) -> dict:
    trimmed = line.rstrip("\r\n").strip()
    if not trimmed:
        raise ValueError("空帧")
    return json.loads(trimmed)


def frame_request(body: dict) -> dict:
    return {"id": str(uuid.uuid4()), "body": body}


def frame_reply(to: str, body: dict) -> dict:
    return {"id": str(uuid.uuid4()), "reply_to": to, "body": body}


class WireError(RuntimeError):
    def __init__(self, kind: str, message: str) -> None:
        super().__init__(message)
        self.kind = kind


def ok_value(frame: dict) -> Any:
    body = frame["body"]
    if body.get("type") == "ok":
        return body.get("value")
    if body.get("type") == "err":
        err = body.get("error") or {}
        raise WireError(str(err.get("kind") or "failed"), str(err.get("message") or ""))
    raise WireError("failed", f"远程应答类型不符: {body}")
