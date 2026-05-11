#!/usr/bin/env python3
"""Dump the surrounding user/assistant turns from the session that
contains the only cargo FAILED hits for the in_flight_semaphore test,
so we can see what was being worked on at the time.
"""

import json
from pathlib import Path

SESSION = Path.home() / (
    ".claude/projects/-home-folk-Programs-ratatoskr"
    "/3fa771c2-a0a8-4d14-92c9-39e084790973.jsonl"
)
TEST = "in_flight_semaphore_caps_concurrent_handlers_and_heartbeat_bypasses"

records = []
for raw in SESSION.open():
    raw = raw.strip()
    if not raw:
        continue
    try:
        records.append(json.loads(raw))
    except json.JSONDecodeError:
        records.append({"_raw": raw})

print(f"Session has {len(records)} records")

# Find records whose serialised form mentions the test.
hits = []
for i, rec in enumerate(records):
    if TEST in json.dumps(rec):
        hits.append(i)

print(f"Records mentioning {TEST}: {len(hits)}")
print(f"Indices: {hits}\n")

for idx in hits[:8]:
    rec = records[idx]
    ts = rec.get("timestamp", "?")
    typ = rec.get("type", "?")
    # Pull a content snippet.
    content = rec.get("message", {}).get("content")
    if isinstance(content, list):
        texts = []
        for c in content:
            if isinstance(c, dict):
                if c.get("type") == "text":
                    texts.append(c.get("text", ""))
                elif c.get("type") == "tool_result":
                    cc = c.get("content")
                    if isinstance(cc, list):
                        for x in cc:
                            if isinstance(x, dict) and x.get("type") == "text":
                                texts.append(x.get("text", ""))
                    elif isinstance(cc, str):
                        texts.append(cc)
                elif c.get("type") == "tool_use":
                    texts.append(f"[tool_use {c.get('name')} input={json.dumps(c.get('input'))[:200]}]")
        body = "\n---\n".join(texts)
    elif isinstance(content, str):
        body = content
    else:
        body = json.dumps(rec)[:1000]

    # Trim & locate the test name in the body.
    pos = body.find(TEST)
    if pos >= 0:
        lo = max(0, pos - 800)
        hi = min(len(body), pos + 1200)
        body = body[lo:hi]

    print(f"=== record {idx} type={typ} ts={ts} ===")
    print(body[:2400])
    print()
