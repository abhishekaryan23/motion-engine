#!/usr/bin/env python3
"""Call one motion-mcp tool over stdio, the way an MCP client does (0.21).

    python3 scripts/mcp_call.py [--profile weak] [--server target/release/motion-mcp] \
        make_video '{"story": {...}}'
    python3 scripts/mcp_call.py --list            # tools/list with sizes

Progress notifications go to stderr; the tool's text reply goes to stdout
(and its structured content with --json). Extra server flags after `--`.
No network: the server is a local child process.
"""
import argparse
import json
import subprocess
import sys
import time


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--profile", default="weak")
    ap.add_argument("--server", default="target/release/motion-mcp")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--json", action="store_true", help="print structured content too")
    ap.add_argument("tool", nargs="?")
    ap.add_argument("args", nargs="?", default="{}")
    ap.add_argument("server_args", nargs=argparse.REMAINDER)
    a = ap.parse_args()
    extra = [x for x in a.server_args if x != "--"]
    proc = subprocess.Popen(
        [a.server, "--profile", a.profile, *extra],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)

    def send(msg):
        proc.stdin.write(json.dumps(msg) + "\n")
        proc.stdin.flush()

    def wait_for(id_):
        while True:
            line = proc.stdout.readline()
            if not line:
                sys.exit("server closed the connection")
            msg = json.loads(line)
            if msg.get("method") == "notifications/progress":
                p = msg["params"]
                print(f"  progress {p.get('progress')}% {p.get('message', '')}", file=sys.stderr)
                continue
            if msg.get("id") == id_:
                return msg

    send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "mcp_call.py", "version": "0.21"}}})
    init = wait_for(1)
    send({"jsonrpc": "2.0", "method": "notifications/initialized"})
    if a.list or not a.tool:
        send({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
        tools = wait_for(2)["result"]["tools"]
        total = 0
        for t in tools:
            n = len(json.dumps(t, separators=(",", ":"), ensure_ascii=False))
            total += n
            print(f"{t['name']:<20} {n} chars")
        print(f"{len(tools)} tools, {total} chars ({init['result']['serverInfo']['name']})")
    else:
        t0 = time.time()
        send({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {
            "name": a.tool, "arguments": json.loads(a.args),
            "_meta": {"progressToken": "p1"}}})
        res = wait_for(3)
        if "error" in res:
            sys.exit(f"error: {res['error']}")
        r = res["result"]
        for c in r.get("content", []):
            if c.get("type") == "text":
                print(c["text"])
            elif c.get("type") == "resource_link":
                print(f"link: {c.get('uri')}")
            elif c.get("type") == "image":
                print(f"image: {len(c.get('data', ''))} base64 chars ({c.get('mimeType')})")
        if a.json:
            print(json.dumps(r.get("structuredContent"), indent=2))
        print(f"({time.time() - t0:.1f} s, isError={r.get('isError', False)})", file=sys.stderr)
    proc.stdin.close()
    proc.wait(timeout=30)


if __name__ == "__main__":
    main()
