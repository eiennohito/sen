#!/usr/bin/env python3
"""Verify read-only members in an owned offline demo.

Agents must launch their owned preview with tools/run-ui-debug.py; do not map
a debug window on the user's desktop. Pass that scope's unique --port here.
"""
import argparse
import json
import time
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
    deadline=time.monotonic()+3
    while True:
        try:
            matches=[row for row in tree() if row.get("accessibleRole") == role and (row.get("accessibleLabel") == label or row.get("accessibleLabel", "").startswith(label + ", "))]
            return next((row for row in matches if row.get("accessibleLabel")==label),matches[0])
        except (StopIteration,IndexError):
            if time.monotonic()>deadline: raise AssertionError(f"Missing {role} {label}") from None
            time.sleep(.05)

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
if call("get_element_properties",elementHandle=root)["size"]["width"] < 740:
    modified("\x12","\uf702")
click("Weekend plans")
click("Members, 25")
element("Close details")
if call("get_element_properties", elementHandle=root)["size"]["width"] < 1060:
    modified("\x11", "\n")
    element("Close details")
click("Search members", "TextInput")
key("\uf701")
key("\uf729")
rows=set()
for _ in range(27):
    rows.update(row["accessibleLabel"] for row in tree() if row.get("accessibleRole")=="Button" and row.get("accessibleLabel", "").startswith("Open member: "))
    key("\uf701")
assert len(rows)==25, f"expected 25 current members, got {len(rows)}"
element("Invited (1)","Text")
value("Search members","葵")
click("Open member: 葵")
element("Chat")
element("Rename")
click("Rename")
value("Name","Aoi test 🌿")
click("Save")
element("Aoi test 🌿","Text")
element("葵","Text")
click("Rename")
value("Name","葵")
click("Save")
element("葵","Text")
key("\x1b")
value("Search members","蓮")
click("Open member: 蓮")
element("Profile","Text")
element("Add friend")
assert not any(r.get("accessibleLabel")=="Chat" for r in tree())
click("Add friend")
element("Rename")
element("Chat")
click("Back")
assert element("Search members","TextInput").get("accessibleValue")=="蓮"
value("Search members","Unknown")
click("Open member: Unknown member")
element("Unknown member","Text")
assert not any(r.get("accessibleLabel") in ("Chat","Rename","Add friend") for r in tree())
key("\x1b")
value("Search members","You")
click("Open member: Offline preview, You")
element("Profile","Text")
assert not any(r.get("accessibleLabel") in ("Chat","Rename","Add friend") for r in tree()), "own profile exposed a write"
key("\x1b")
key("\x1b")
assert element("Members, 25").get("accessibleDescription")=="Focused"
click("Weekend plans")
element("Group info","Text")
click("Notifications","Checkbox")
deadline=time.monotonic()+3
while element("Notifications","Checkbox").get("accessibleValue") != "Off":
    assert time.monotonic()<deadline, "mute did not update the switch"
    time.sleep(.02)
click("Leave group")
element("Leave group?","Text")
key("\x1b")
element("Leave group")
click("Leave group")
click("Leave")
assert not any(r.get("accessibleLabel")=="Leave group" for r in tree())
click("Close details")
assert not element("Send").get("accessibleEnabled"), "left group remained writable"
before=element("Message draft","TextInput").get("accessibleValue","")
click("Message draft","TextInput")
key("cannot type after leaving")
assert element("Message draft","TextInput").get("accessibleValue","")==before, "left group draft remained editable"
print("PASS: current members + invitees; friend, non-friend, own/unknown actions; Rename/Add friend in place; Back/Escape/opener focus; mute; Leave cancel/confirm; read-only chat")
