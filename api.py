from __future__ import annotations

from datetime import datetime, timezone
from typing import Any
from uuid import uuid4

from fastapi import FastAPI, Query
from pydantic import BaseModel

app = FastAPI(title="metarom-api", version="0.1.0")
MEMORY: list[dict[str, Any]] = []


class MemoryWriteRequest(BaseModel):
    agent_id: str = "system"
    event_type: str = "note"
    payload: dict[str, Any] = {}
    ttl: int | None = None


@app.get("/health")
def health() -> dict[str, Any]:
    return {
        "status": "ok",
        "service": "metarom-api",
        "time": datetime.now(timezone.utc).isoformat(),
        "memory_count": len(MEMORY),
    }


@app.get("/skills")
def skills() -> dict[str, Any]:
    return {
        "id": "metarom",
        "capabilities": [
            "memory_write",
            "memory_read",
            "memory_search",
            "rom_snapshot",
            "rom_replay",
        ],
    }


@app.post("/memory/write")
def memory_write(req: MemoryWriteRequest) -> dict[str, Any]:
    item = {
        "id": str(uuid4()),
        "agent_id": req.agent_id,
        "event_type": req.event_type,
        "payload": req.payload,
        "ttl": req.ttl,
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }
    MEMORY.append(item)
    return {"ok": True, "item": item}


@app.get("/memory/read")
def memory_read(limit: int = Query(default=20, ge=1, le=100)) -> dict[str, Any]:
    return {"items": list(reversed(MEMORY[-limit:]))}


@app.get("/memory/search")
def memory_search(query: str = Query(default=""), limit: int = Query(default=5, ge=1, le=20)) -> dict[str, Any]:
    needle = query.lower().strip()
    if not needle:
        return {"items": list(reversed(MEMORY[-limit:]))}

    matches = []
    for item in reversed(MEMORY):
        haystack = f"{item.get('agent_id','')} {item.get('event_type','')} {item.get('payload',{})}".lower()
        if needle in haystack:
            matches.append(item)
        if len(matches) >= limit:
            break
    return {"items": matches}
