#!/usr/bin/env python3
"""Verify the offline browser filters, metadata and keyboard path.

Agents must launch their owned preview with tools/run-ui-debug.py; do not map
a debug window on the user's desktop. Pass that scope's unique --port here.
"""
import argparse
import json
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--port", type=int, default=9453)
args = parser.parse_args()
sequence = 0

def rpc(method, params):
    global sequence
    sequence += 1
    request = urllib.request.Request(f"http://127.0.0.1:{args.port}/mcp", data=json.dumps({"jsonrpc": "2.0", "id": sequence, "method": method, "params": params}).encode(), headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream"})
    with urllib.request.urlopen(request, timeout=10) as response:
        reply = json.load(response)
    assert "error" not in reply, reply
    return reply["result"]

def call(name, **arguments):
    result = rpc("tools/call", {"name": name, "arguments": arguments})
    assert not result.get("isError"), result
    return json.loads(result["content"][0]["text"])

rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "line-browser-check", "version": "1"}})
window = call("list_windows")["windowHandles"][0]
root = call("get_window_properties", windowHandle=window)["rootElementHandle"]

def tree():
    result = call("get_element_tree", elementHandle=root, maxElements=700)
    assert not result.get("truncated")
    return result["elements"]

def element(label, role="Button"):
    return next(row for row in tree() if row.get("accessibleRole") == role and (row.get("accessibleLabel") == label or row.get("accessibleLabel", "").startswith(label + ", ")))

def click(label, role="Button"):
    call("click_element", elementHandle=element(label, role)["handle"])

def key(text, event_type="PressAndRelease"):
    call("dispatch_key_event", windowHandle=window, text=text, eventType=event_type)

def modified(modifier, text):
    key(modifier, "Press")
    key(text)
    key(modifier, "Release")

def value(label, text):
    call("set_element_value", elementHandle=element(label, "TextInput")["handle"], value=text)

def chats():
    return [row for row in tree() if row.get("accessibleRole") == "Button" and row.get("accessibleLabel", "").startswith(("葵, ", "Weekend plans, ", "Notes to self, "))]

# This marker exists exclusively in --demo; never touch an account window.
key("\uf70f")  # F12
demo_marker = element("Synthetic interaction scenarios", "Text")
assert any(part.get("id", "").endswith("::demo-marker") for part in demo_marker.get("typeNamesAndIds", [])), "refusing a window without the compiled demo-only marker"
click("Close demo scenarios")
click("Chats")
click("All")
value("Find a conversation", "")
click("葵")
value("Message draft", "List draft 日本語")
assert "Draft: List draft" in element("葵")["accessibleLabel"]
element("Draft", "Image")
click("Groups")
assert len(chats()) == 1 and chats()[0]["accessibleLabel"].startswith("Weekend plans, ")
assert element("Message draft", "TextInput").get("accessibleValue") == "List draft 日本語"
friend_chip = next(row for row in tree() if row.get("accessibleLabel") == "Friends" and row.get("accessibleRole") == "Button" and row.get("absolutePosition", {}).get("x", 0) >= 56)
call("click_element", elementHandle=friend_chip["handle"])
assert len(chats()) == 2 and not any(row["accessibleLabel"].startswith("Weekend plans, ") for row in chats())
# Select All explicitly and exercise the global shortcuts from the composer.
click("All")
click("Message draft", "TextInput")
key("\uf70f")
key("\x1b")
assert element("Message draft", "TextInput").get("accessibleDescription") == "Focused message draft"
modified("\x11", "1")
element("Search loaded friends and groups", "TextInput")
modified("\x11", "2")
element("Find a conversation", "TextInput")
click("葵")
modified("\x12", "\uf701")
assert element("Message draft", "TextInput").get("accessibleValue", "") == "", "Alt+Down did not switch conversation"
modified("\x12", "\uf700")
assert element("Message draft", "TextInput").get("accessibleValue") == "List draft 日本語", "Alt+Up lost the previous draft"
modified("\x11", "f")
element("Find a conversation", "TextInput")
key("\t")
key("\uf729")  # Home
key("\uf72b")  # End
key("\n")
assert element("Message draft", "TextInput").get("accessibleValue", "") == "", "list End/Enter did not open the final row"
print("PASS: category filtering, draft marker, Ctrl+1 Friends/Ctrl+2 Chats, Alt+Up/Down, Ctrl+F and one-stop list Home/End/Enter")
