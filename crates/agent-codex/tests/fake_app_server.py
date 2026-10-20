#!/usr/bin/env python3
# A fake `codex app-server` for Kaluta's adapter tests. Newline-delimited
# JSON-RPC without "jsonrpc", like the real one. Records what it receives.
import json, os, sys

log_dir = os.path.dirname(os.path.abspath(__file__))
if len(sys.argv) > 1 and sys.argv[1] == "--version":
    print("codex-cli 0.150.0"); sys.exit(0)
with open(os.path.join(log_dir, "argv.json"), "w") as f:
    json.dump(sys.argv[1:], f)
received = open(os.path.join(log_dir, "received.jsonl"), "w")

def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n"); sys.stdout.flush()

thread = "thr_1"
turns = 0
for line in sys.stdin:
    msg = json.loads(line)
    received.write(line); received.flush()
    method = msg.get("method")
    if method == "initialize":
        send({"id": msg["id"], "result": {"userAgent": "codex/0.150.0"}})
    elif method in ("thread/start", "thread/resume"):
        thread = msg["params"].get("threadId", thread)
        send({"id": msg["id"], "result": {"thread": {"id": thread}}})
    elif method == "model/list":
        # Two pages, as the real one may answer; a hidden model is left out.
        if msg["params"].get("cursor") is None:
            send({"id": msg["id"], "result": {"data": [
                {"id": "gpt-old", "model": "gpt-old", "displayName": "GPT Old", "isDefault": False, "hidden": True},
                {"id": "gpt-5.5-mini", "model": "gpt-5.5-mini", "displayName": "GPT-5.5 Mini", "isDefault": False, "hidden": False}],
                "nextCursor": "page2"}})
        else:
            send({"id": msg["id"], "result": {"data": [
                {"id": "gpt-5.5", "model": "gpt-5.5", "displayName": "GPT-5.5", "isDefault": True, "hidden": False}],
                "nextCursor": None}})
    elif method == "turn/start":
        turns += 1
        turn = "turn_%d" % turns
        send({"id": msg["id"], "result": {"turn": {"id": turn, "status": "inProgress", "items": []}}})
        text = msg["params"]["input"][0]["text"]
        if text.endswith("hang"):
            continue
        # The model Codex is set to is refused for this account, unless the
        # turn names another: as a ChatGPT account answers for gpt-6.1-sol.
        if text.endswith("refuse") and not msg["params"].get("model"):
            body = json.dumps({"type": "error", "status": 400, "error": {"type": "invalid_request_error",
                "message": "The 'gpt-6.1-sol' model is not supported when using Codex with a ChatGPT account."}})
            send({"method": "error", "params": {"threadId": thread, "turnId": turn, "willRetry": False, "error": {"message": body}}})
            send({"method": "turn/completed", "params": {"threadId": thread, "turn": {"id": turn, "status": "failed", "items": [],
                "error": {"message": body}}}})
            continue
        send({"id": 900, "method": "item/commandExecution/requestApproval", "params": {"threadId": thread}})
        # Like Codex, wait for the answer before going on, so it is recorded
        # before turn/completed: the test reads the record once the turn ends.
        answer = sys.stdin.readline()
        received.write(answer); received.flush()
        send({"method": "item/started", "params": {"threadId": thread, "turnId": turn, "startedAtMs": 0, "item": {
            "type": "mcpToolCall", "id": "call_1", "server": "kaluta", "tool": "mail_search",
            "arguments": {"query": "is:unread"}, "status": "inProgress"}}})
        send({"method": "item/completed", "params": {"threadId": thread, "turnId": turn, "completedAtMs": 0, "item": {
            "type": "mcpToolCall", "id": "call_1", "server": "kaluta", "tool": "mail_search",
            "arguments": {"query": "is:unread"}, "status": "completed",
            "result": {"content": [{"type": "text", "text": "{\"threads\":[]}"}]}}}})
        send({"method": "item/reasoning/summaryTextDelta", "params": {"threadId": thread, "turnId": turn, "itemId": "r", "summaryIndex": 0, "delta": "Checking."}})
        send({"method": "item/agentMessage/delta", "params": {"threadId": thread, "turnId": turn, "itemId": "m", "delta": "No unread "}})
        send({"method": "item/agentMessage/delta", "params": {"threadId": "someone-else", "turnId": turn, "itemId": "m", "delta": "IGNORED"}})
        send({"method": "item/agentMessage/delta", "params": {"threadId": thread, "turnId": turn, "itemId": "m", "delta": "mail."}})
        send({"method": "thread/tokenUsage/updated", "params": {"threadId": thread, "turnId": turn, "tokenUsage": {
            "last": {"inputTokens": 50, "outputTokens": 7, "cachedInputTokens": 10, "totalTokens": 57,
                     "reasoningOutputTokens": 0, "cacheWriteInputTokens": 0},
            "total": {"inputTokens": 50, "outputTokens": 7, "cachedInputTokens": 10, "totalTokens": 57,
                      "reasoningOutputTokens": 0, "cacheWriteInputTokens": 0}}}})
        send({"method": "turn/completed", "params": {"threadId": thread, "turn": {"id": turn, "status": "completed", "items": []}}})
    elif method == "turn/interrupt":
        send({"id": msg["id"], "result": {}})
        send({"method": "turn/completed", "params": {"threadId": thread, "turn": {"id": msg["params"]["turnId"], "status": "interrupted", "items": []}}})
