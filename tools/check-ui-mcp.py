#!/usr/bin/env python3
"""Exercise an already-running --demo window through Slint's optional MCP server."""
import argparse
import base64
import json
import time
from pathlib import Path
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--port", type=int, default=9447)
parser.add_argument("--screenshot", type=Path)
parser.add_argument("--scenarios", action="store_true", help="Run synthetic arrival, media, clipboard, and uncertain-send scenarios")
parser.add_argument("--friends", action="store_true", help="Exercise offline friend search, profiles, categories and opening chats")
parser.add_argument("--auth-recovery", action="store_true", help="Exercise the offline failed-login recovery UI without pairing")
parser.add_argument("--layout-photos", action="store_true", help="Check bounded avatars/media, settings placement, and photo viewer")
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
before_shortcut = value.get("accessibleValue")
call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Press")
call("dispatch_key_event", windowHandle=window, text="\n")
call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Release")
assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == before_shortcut, "disabled Ctrl+Enter changed the draft"
if args.scenarios:
    def key(text, event_type="PressAndRelease"):
        call("dispatch_key_event", windowHandle=window, text=text, eventType=event_type)

    def ctrl(text):
        key("\x11", "Press")
        key(text)
        key("\x11", "Release")

    def bodies():
        return [item for item in tree() if item.get("accessibleLabel") in ("Incoming message", "Outgoing message")]

    call("click_element", elementHandle=element("Long history", "Button")["handle"])
    time.sleep(0.05)
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 600, "y": 280}, deltaY=800, deltaX=0)
    time.sleep(0.05)
    element("Latest", "Button")  # Older loaded messages below are not new arrivals.
    body = next(item for item in bodies() if "\n" in item.get("accessibleValue", "") and item["absolutePosition"]["y"] > 110)
    expected_text = body["accessibleValue"]
    expected_y = body["absolutePosition"]["y"]
    call("click_element", elementHandle=element("Load media", "Button")["handle"])
    call("click_element", elementHandle=element("Receive 3", "Button")["handle"])
    call("click_element", elementHandle=body["handle"])
    ctrl("a")
    time.sleep(2.3)
    preserved = call("get_element_properties", elementHandle=body["handle"])
    assert preserved["absolutePosition"]["y"] == expected_y, "arrivals/media moved the reading anchor"
    element("Latest · 3 new", "Button")
    ctrl("c")
    call("set_element_value", elementHandle=composer, value="")
    call("click_element", elementHandle=composer)
    ctrl("v")
    copied = call("get_element_properties", elementHandle=composer).get("accessibleValue")
    assert copied == expected_text, "clipboard lost selected text/emoji or included metadata"
    call("click_element", elementHandle=element("Latest · 3 new", "Button")["handle"])
    time.sleep(0.05)
    assert not any("Latest" in item.get("accessibleLabel", "") for item in tree() if item.get("accessibleRole") == "Button"), "Latest did not resume following"
    call("click_element", elementHandle=element("Receive 3", "Button")["handle"])
    time.sleep(0.5)
    assert any("到着 3" in item.get("accessibleValue", "") for item in bodies()), "at-latest burst did not remain visible"
    assert not any("Latest" in item.get("accessibleLabel", "") for item in tree() if item.get("accessibleRole") == "Button"), "at-latest burst disabled following"
    call("set_element_value", elementHandle=composer, value="Keep this newer draft")
    call("click_element", elementHandle=element("Uncertain send", "Button")["handle"])
    time.sleep(1.2)
    assert any("Confirmation unavailable" in item.get("accessibleLabel", "") for item in tree()), "uncertain result was not attached to its outgoing item"
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == "Keep this newer draft", "late result replaced newer text"
    call("click_element", elementHandle=element("Append text to draft", "Button")["handle"])
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue", "").startswith("Keep this newer draft\n"), "recovery overwrote the draft"
    print("PASS: simulated pointer scrolling, burst anchor/count, delayed media, keyboard selection/clipboard, uncertain-send recovery")

if args.layout_photos:
    assert not any(item.get("accessibleLabel") in ("Smaller text", "Larger text", "Text size") for item in tree()), "text-size controls leaked onto the main screen"
    avatars = [item for item in tree() if item.get("accessibleLabel") == "Contact photo"]
    assert avatars and all(item["size"] == {"height": 40.0, "width": 40.0} for item in avatars), "avatar escaped its frame"
    call("click_element", elementHandle=element("Settings", "Button")["handle"])
    element("Message text size", "Combobox")
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    element("Settings", "Button")
    before = call("get_element_properties", elementHandle=composer).get("accessibleValue")
    call("click_element", elementHandle=element("Photo preview", "Button")["handle"])
    time.sleep(0.05)
    preview = next(item for item in tree() if item.get("accessibleLabel") == "Photo preview" and item.get("accessibleRole") == "Image")
    timestamp = next(item for item in tree() if item.get("accessibleRole") == "Text" and item.get("accessibleLabel", "").startswith("2026/10/15 12:05"))
    assert preview["size"]["height"] == 180, "photo exceeded reserved preview height"
    assert timestamp["absolutePosition"]["x"] >= preview["absolutePosition"]["x"] + preview["size"]["width"], "timestamp overlapped the photo instead of sitting beside it"
    assert timestamp["size"]["height"] <= 16, "timestamp occupied multiple rows"
    call("click_element", elementHandle=element("Open photo", "Button")["handle"])
    element("Close photo", "Button")
    assert any("Synthetic original · 1200 × 800" in item.get("accessibleLabel", "") for item in tree()), "viewer did not request original resolution"
    call("click_element", elementHandle=element("Original size", "Button")["handle"])
    original = element("Photo in viewer", "Image")
    assert original["size"]["width"] >= 1199, "original-size view only enlarged the thumbnail"
    call("click_element", elementHandle=element("Fit", "Button")["handle"])
    assert element("Photo in viewer", "Image")["size"]["width"] < original["size"]["width"], "Fit did not restore a bounded view"
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == before, "viewer changed the draft"
    element("Open photo", "Button")
    print("PASS: bounded round-avatar frames, settings placement, media/timestamp separation, original viewer, zoom/fit, Escape, and draft retention")
    call("click_element", elementHandle=element("Sticker preview", "Button")["handle"])
    time.sleep(0.05)
    element("[sticker]", "Text")
    before_sticker_time = element("2026/10/15 12:09", "Text")["absolutePosition"]
    time.sleep(1.1)
    element("Message image", "Image")
    assert not any(item.get("accessibleLabel") == "[sticker]" or item.get("accessibleValue") == "[sticker]" for item in tree()), "loaded sticker retained its fallback label"
    assert element("2026/10/15 12:09", "Text")["absolutePosition"] == before_sticker_time, "sticker completion moved its timestamp"
    print("PASS: sticker fallback disappears after loading without moving the timestamp")
    call("click_element", elementHandle=element("Group preview", "Button")["handle"])
    time.sleep(0.05)
    grouped = tree()
    labels = [item.get("accessibleLabel", "") for item in grouped]
    assert labels.count("Sender: 葵") == 2 and labels.count("Sender: 蓮") == 1, "sender names were missing or repeated within a group"
    sender_avatars = [item for item in grouped if item.get("accessibleRole") == "Image" and item.get("accessibleLabel") in ("葵 avatar", "蓮 avatar")]
    assert len(sender_avatars) == 3 and all(item["size"] == {"width": 32.0, "height": 32.0} for item in sender_avatars), "sender avatar grouping or bounds failed"
    assert labels.count("2026/10/15") == 1 and labels.count("2026/10/16") == 1, "calendar dates were repeated or omitted"
    # The last group's time can be below the viewport. Inspect both positions
    # instead of assuming accessibility exposes offscreen labels.
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 600, "y": 280}, deltaY=-800, deltaX=0)
    time.sleep(0.05)
    scrolled_group = tree()
    times = set()
    for visible_group in (grouped, scrolled_group):
        message_bodies = [item for item in visible_group if item.get("accessibleLabel") in ("Incoming message", "Outgoing message")]
        for meta in visible_group:
            label = meta.get("accessibleLabel", "")
            if not (label.startswith("2026/") and " " in label):
                continue
            times.add(label)
            assert any(meta["absolutePosition"]["x"] >= body["absolutePosition"]["x"] + body["size"]["width"] and abs(meta["absolutePosition"]["y"] - body["absolutePosition"]["y"]) <= body["size"]["height"] + 12 for body in message_bodies), "group time added a footer row"
    assert sorted(times) == ["2026/10/15 09:18", "2026/10/15 09:20", "2026/10/15 09:21", "2026/10/15 09:27", "2026/10/16 09:18"], "five-minute/sender/day grouping showed the wrong timestamps"
    print("PASS: sender avatars/names, five-minute grouping, day boundaries, and compact time/status beside bubbles")

if args.friends:
    call("set_element_value", elementHandle=composer, value=draft)
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    friend_search = element("Search loaded friends and groups", "TextInput")["handle"]
    call("set_element_value", elementHandle=friend_search, value="葵")
    friend_rows = [item for item in tree() if item.get("accessibleLabel", "").startswith("Open profile:")]
    assert len(friend_rows) == 1, "friend name search failed"
    call("click_element", elementHandle=friend_search)
    call("dispatch_key_event", windowHandle=window, text="\uf701")  # Down arrow
    call("dispatch_key_event", windowHandle=window, text="\n")
    element("Friend profile", "Text")
    assert not any(item.get("accessibleLabel") == "Message draft" for item in tree()), "profile exposed background composer"
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    element("Search loaded friends and groups", "TextInput")
    call("click_element", elementHandle=element("Chats", "Button")["handle"])
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == draft, "friend browsing lost draft"
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    assert call("get_element_properties", elementHandle=friend_search).get("accessibleValue") == "葵", "section switch lost friend query"
    call("set_element_value", elementHandle=friend_search, value="")
    call("click_element", elementHandle=element("Groups ▾", "Button")["handle"])
    assert not any(item.get("accessibleLabel", "").startswith("Open profile:") and "Weekend plans" in item.get("accessibleLabel", "") for item in tree()), "collapsed groups remained visible"
    call("click_element", elementHandle=element("Groups ▸", "Button")["handle"])
    call("click_element", elementHandle=element("Open profile: 新しい友だち 🌸", "Button")["handle"])
    call("click_element", elementHandle=element("Open chat", "Button")["handle"])
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue", "") == "", "new friend inherited another draft"
    call("click_element", elementHandle=element("Chats", "Button")["handle"])
    call("click_element", elementHandle=element("葵", "Button")["handle"])
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == draft, "opening a new friend lost prior draft"
    print("PASS: friend search keyboard activation, profile Escape, retained query/categories, new-chat isolation and prior draft")

if args.auth_recovery:
    before = call("get_element_properties", elementHandle=composer).get("accessibleValue")
    call("click_element", elementHandle=element("Login recovery", "Button")["handle"])
    element("Retry connection", "Button")
    call("click_element", elementHandle=element("Sign in again", "Button")["handle"])
    assert any("Chrome device slot" in item.get("accessibleLabel", "") for item in tree()), "sign-in did not explain its scope"
    assert any("encrypted drafts" in item.get("accessibleLabel", "") for item in tree()), "sign-in did not explain draft isolation"
    call("click_element", elementHandle=element("Start sign-in", "Button")["handle"])
    assert any("No pairing request was made" in item.get("accessibleLabel", "") for item in tree()), "offline sign-in guard did not report its result"
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    element("Sign in again", "Button")
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == before, "sign-in dismissal changed the draft"
    assert element("Retry connection", "Button").get("accessibleEnabled"), "retry stayed disabled"
    print("PASS: failed-login recovery controls, explicit sign-in explanation, offline pairing guard, Escape, and draft preservation")

if args.screenshot:
    result = rpc("tools/call", {"name": "take_screenshot", "arguments": {"windowHandle": window}})
    image = next(item for item in result["content"] if item["type"] == "image")
    args.screenshot.write_bytes(base64.b64decode(image["data"]))
print(f"PASS: MCP draft retention, chat selection, filtering, disabled offline Send, and multiline input; scale {properties.get('scaleFactor')}")
