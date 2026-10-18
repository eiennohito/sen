#!/usr/bin/env python3
"""Verify read-only members in an owned offline demo.

Agents must launch their owned preview with tools/run-ui-debug.py; do not map
a debug window on the user's desktop. Pass that scope's unique --port here.
"""
import argparse
import json
import urllib.request

from mcp_preview import add_preview_arguments, launch_preview

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--port", type=int, default=9453)
add_preview_arguments(parser)
args = parser.parse_args()
launch_preview(args)
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

rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "line-members-check", "version": "1"}})
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


def scenario_element(label):
    # Inspect the modal itself with the same bound as the shell behind it.
    handles = call("find_elements_by_id", windowHandle=window, elementsId="DemoScenarios::scenario-panel")["elementHandles"]
    assert len(handles) == 1, "demo dialog was not unique"
    result = call("get_element_tree", elementHandle=handles[0], maxElements=500)
    assert not result.get("truncated"), "demo dialog exceeded test bound"
    return next(item for item in result["elements"] if item.get("accessibleLabel") == label and item.get("accessibleRole") == "Button")

def scenario_action(label):
    try:
        action = scenario_element(label)
    except StopIteration:
        close = scenario_element("Close demo scenarios")
        position = {"x": close["absolutePosition"]["x"] + close["size"]["width"] / 2, "y": close["absolutePosition"]["y"] + close["size"]["height"] + 60}
        for delta in (10000, *([-320] * 12)):
            call("dispatch_pointer_scroll", windowHandle=window, position=position, deltaY=delta)
            try:
                action = scenario_element(label)
                break
            except StopIteration:
                pass
        else:
            raise AssertionError(f"demo scenario was not reachable: {label}") from None
    return action

def demo_scenario(label):
    global scenario_bounds
    call("dispatch_key_event", windowHandle=window, text="\uf70f")  # F12
    if label == "Load media":
        call("click_element", elementHandle=scenario_action("Media failures")["handle"])
    call("click_element", elementHandle=scenario_action(label)["handle"])



key("\uf70f")
marker = element("Synthetic interaction scenarios", "Text")
assert any(part.get("id", "").endswith("::demo-marker") for part in marker.get("typeNamesAndIds", [])), "refusing a non-demo window"
click("Close demo scenarios")
demo_scenario("Group members")
element("Close members")
# A sheet must consume Ctrl+Enter before its focused Close button activates.
if call("get_element_properties", elementHandle=root)["size"]["width"] < 1060:
    modified("\x11", "\n")
    element("Close members")
count = int(element("Members (25)", "Text")["accessibleLabel"].split("(")[1].split(")")[0])
click("Search members", "TextInput")
key("\uf701")
key("\uf729")  # Home
rows = set()
for _ in range(count + 1):
    rows.update(row["accessibleLabel"] for row in tree() if row.get("accessibleRole") == "Button" and row.get("accessibleLabel", "").startswith("Open member: "))
    key("\uf701")
assert len(rows) == count, f"list has {len(rows)} rows; header has {count}"
key("\uf729")
value("Search members", "葵")
click("Open member: 葵")
assert element("Open chat").get("accessibleEnabled"), "friend profile lost Open chat"
click("Back to members")
value("Search members", "蓮")
member = element("Open member: 蓮")
assert not any(row.get("accessibleLabel", "").startswith("Open member: Weekend") for row in tree()), "search did not filter"
call("click_element", elementHandle=member["handle"])
element("Member profile", "Text")
element("蓮", "Text")
assert not any(row.get("accessibleLabel") == "Open chat" for row in tree()), "non-friend without a chat exposed Open chat"
click("Back to members")
assert element("Search members", "TextInput").get("accessibleValue") == "蓮", "Back lost query"
key("\x1b")
header = element("Members, 25")
assert header.get("accessibleDescription") == "Focused", "Escape failed to return header focus"
key("\n")
element("Close members")
value("Search members", "Unknown")
click("Open member: Unknown member")
element("Unknown member", "Text")
assert not any(row.get("accessibleLabel") == "Open chat" for row in tree()), "unknown member exposed Open chat"
key("\x1b")
element("Close members")
assert element("Search members", "TextInput").get("accessibleValue") == "Unknown"
key("\x1b")
assert element("Members, 25").get("accessibleDescription") == "Focused"
key("\n")
value("Search members", "You")
element("Open member: Offline preview, You")
key("\x1b")
# Connection recovery must remain above both member surfaces.
demo_scenario("Offline")
click("Members, 25")
strip = element("Connection needs attention", "Banner")
field = element("Search members", "TextInput")
wide = call("get_element_properties", elementHandle=root)["size"]["width"] >= 1060
if wide:
    assert element("Close members")["absolutePosition"]["y"] < strip["absolutePosition"]["y"], "wide panel acquired a strip gap"
else:
    assert field["absolutePosition"]["y"] >= strip["absolutePosition"]["y"] + strip["size"]["height"], "members search covered by strip"
click("Open member: Offline preview, You")
back = element("Back to members")
if not wide:
    assert back["absolutePosition"]["y"] >= strip["absolutePosition"]["y"] + strip["size"]["height"], "profile covered by strip"
key("\x1b")
element("Close members")
key("\x1b")
assert element("Members, 25").get("accessibleDescription") == "Focused"
print("PASS: list rows equal header count; read-only profile actions; Escape goes Back then closes; Members, 25; search; known/unknown/self profiles; Back preserves query; Escape returns header focus; members and profile below strip")
