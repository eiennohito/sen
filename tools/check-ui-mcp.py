#!/usr/bin/env python3
"""Exercise an already-running --demo window through Slint's optional MCP server."""
import argparse
import base64
import json
from pathlib import Path
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--port", type=int, default=9447)
parser.add_argument("--screenshot", type=Path)
args = parser.parse_args()
counter = 0

def rpc(method, params):
    global counter
    counter += 1
    request = urllib.request.Request(
        f"http://127.0.0.1:{args.port}/mcp",
        data=json.dumps({"jsonrpc": "2.0", "id": counter, "method": method, "params": params}).encode(),
        headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream"},
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        result = json.load(response)
    if "error" in result:
        raise RuntimeError("MCP request failed")
    return result["result"]

def call(name, **arguments):
    result = rpc("tools/call", {"name": name, "arguments": arguments})
    if result.get("isError"):
        raise RuntimeError(f"MCP tool failed: {name}")
    return json.loads(result["content"][0]["text"])

rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "line-ui-check", "version": "1"}})
window = call("list_windows")["windowHandles"][0]
properties = call("get_window_properties", windowHandle=window)
root = properties["rootElementHandle"]

def tree():
    result = call("get_element_tree", elementHandle=root, maxElements=500)
    assert not result.get("truncated"), "UI tree exceeded test bound"
    return result["elements"]

def element(label, role):
    return next(item for item in tree() if item.get("accessibleLabel") == label and item.get("accessibleRole") == role)

# Refuse to operate on a real account. This script never clicks Send.
element("Offline preview", "Text")
element("Messages are examples", "Text")
composer = element("Message draft", "TextInput")["handle"]
search = element("Find a conversation", "TextInput")["handle"]
draft = "日本語の下書き\nDraft with emoji 🐈"
call("set_element_value", elementHandle=composer, value=draft)
call("click_element", elementHandle=element("Weekend plans", "Button")["handle"])
call("set_element_value", elementHandle=composer, value="Another draft")
call("click_element", elementHandle=element("葵", "Button")["handle"])
value = call("get_element_properties", elementHandle=composer)
assert value.get("accessibleValue") == draft, "draft did not survive chat switching"
call("set_element_value", elementHandle=search, value="Weekend")
chat_rows = [item for item in tree() if any(base.get("id") == "AppWindow::row-focus" for base in item.get("typeNamesAndIds", []))]
assert len(chat_rows) == 1 and chat_rows[0]["accessibleLabel"] == "Weekend plans", "search did not filter chats"
call("set_element_value", elementHandle=search, value="")
assert not element("Send", "Button").get("accessibleEnabled", False), "offline Send became enabled"
call("click_element", elementHandle=composer)
call("dispatch_key_event", windowHandle=window, text="\n")
value = call("get_element_properties", elementHandle=composer)
assert value.get("accessibleValue", "").endswith("\n"), "Enter did not insert a newline"
if args.screenshot:
    result = rpc("tools/call", {"name": "take_screenshot", "arguments": {"windowHandle": window}})
    image = next(item for item in result["content"] if item["type"] == "image")
    args.screenshot.write_bytes(base64.b64decode(image["data"]))
print(f"PASS: MCP draft retention, chat selection, filtering, disabled offline Send, and multiline input; scale {properties.get('scaleFactor')}")
