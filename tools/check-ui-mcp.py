#!/usr/bin/env python3
"""Exercise an already-running --demo window through Slint's optional MCP server.

Agents must launch their owned preview with tools/run-ui-debug.py; do not map
a debug window on the user's desktop. Pass that scope's unique --port here.
"""
import argparse
import base64
import json
import time
import datetime
from pathlib import Path
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--port", type=int, default=9447)
parser.add_argument("--screenshot", type=Path)
parser.add_argument("--unread", action="store_true", help="Check synthetic unread badges, divider and arrivals while reading older history")
parser.add_argument("--paging", action="store_true", help="Exercise bounded thousand-message older paging, failure and retry")
parser.add_argument("--paging-pid", type=int, help="Optional demo process PID for RSS sampling")
parser.add_argument("--scenarios", action="store_true", help="Run synthetic arrival, media, clipboard, and uncertain-send scenarios")
parser.add_argument("--friends", action="store_true", help="Exercise offline friend search, profiles, categories and opening chats")
parser.add_argument("--auth-recovery", action="store_true", help="Exercise the offline failed-login recovery UI without pairing")
parser.add_argument("--layout-photos", action="store_true", help="Check bounded avatars/media, settings placement, and photo viewer")
parser.add_argument("--reading", action="store_true", help="Check reading focus, keyboard scroll, stable loading, links and content placeholders")
parser.add_argument("--rich", action="store_true", help="Check rich emoji fallback copy, reply jump/return, durations and file Save")
parser.add_argument("--layers", action="store_true", help="Check dialogs, profile panels, viewer input, export, and opener focus")
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
    result = call("get_element_tree", elementHandle=root, maxElements=700)
    assert not result.get("truncated"), "UI tree exceeded test bound"
    return result["elements"]

def element(label, role):
    return next(item for item in tree() if (item.get("accessibleLabel") == label or item.get("accessibleLabel", "").startswith(label + ", ")) and item.get("accessibleRole") == role)

def demo_scenario(label):
    call("dispatch_key_event", windowHandle=window, text="\uf70f")  # F12
    try:
        action = element(label, "Button")
    except StopIteration:
        close = element("Close demo scenarios", "Button")
        position = {"x": close["absolutePosition"]["x"] + close["size"]["width"] / 2, "y": close["absolutePosition"]["y"] + close["size"]["height"] + 60}
        for delta in (10000, -320, -320, -320):
            call("dispatch_pointer_scroll", windowHandle=window, position=position, deltaY=delta)
            try:
                action = element(label, "Button")
                break
            except StopIteration:
                pass
        else:
            raise AssertionError(f"demo scenario was not reachable: {label}") from None
    call("click_element", elementHandle=action["handle"])

# Refuse to operate on a real account. This script never clicks Send.
call("dispatch_key_event", windowHandle=window, text="\uf70f")  # F12
demo_marker = element("Synthetic interaction scenarios", "Text")
assert any(part.get("id", "").endswith("::demo-marker") for part in demo_marker.get("typeNamesAndIds", [])), "refusing a window without the compiled demo-only marker"
call("click_element", elementHandle=element("Close demo scenarios", "Button")["handle"])
deadline = time.monotonic() + 5
while True:
    try:
        composer = element("Message draft", "TextInput")["handle"]
        break
    except StopIteration:
        assert time.monotonic() < deadline, "composer did not appear within 5 seconds"
        time.sleep(0.05)
search = element("Find a conversation", "TextInput")["handle"]
draft = "日本語の下書き\nDraft with emoji 🐈"
call("click_element", elementHandle=element("葵", "Button")["handle"])
call("set_element_value", elementHandle=composer, value=draft)
call("click_element", elementHandle=element("Weekend plans", "Button")["handle"])
call("set_element_value", elementHandle=composer, value="Another draft")
call("click_element", elementHandle=element("葵", "Button")["handle"])
value = call("get_element_properties", elementHandle=composer)
assert value.get("accessibleValue") == draft, "draft did not survive chat switching"
call("set_element_value", elementHandle=search, value="Weekend")
chat_rows = [item for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith(("葵, ", "Weekend plans, ", "Notes to self, "))]
assert len(chat_rows) == 1 and chat_rows[0]["accessibleLabel"].startswith("Weekend plans, "), "search did not filter chats"
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
        return [item for item in tree() if item.get("accessibleRole") == "TextInput" and any(base.get("id", "").endswith("::body") for base in item.get("typeNamesAndIds", []))]

    demo_scenario("Long history")
    time.sleep(0.05)
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 600, "y": 280}, deltaY=800, deltaX=0)
    time.sleep(0.05)
    element("Latest", "Button")  # Older loaded messages below are not new arrivals.
    body = next(item for item in bodies() if "\n" in item.get("accessibleValue", "") and item["absolutePosition"]["y"] > 110)
    expected_text = body["accessibleValue"]
    expected_y = body["absolutePosition"]["y"]
    demo_scenario("Load media")
    demo_scenario("Receive 3")
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
    demo_scenario("Receive 3")
    time.sleep(0.5)
    assert any("到着 3" in item.get("accessibleValue", "") for item in bodies()), "at-latest burst did not remain visible"
    assert not any("Latest" in item.get("accessibleLabel", "") for item in tree() if item.get("accessibleRole") == "Button"), "at-latest burst disabled following"
    call("set_element_value", elementHandle=composer, value="Keep this newer draft")
    demo_scenario("Uncertain send")
    time.sleep(1.2)
    assert any("Confirmation unavailable" in item.get("accessibleLabel", "") for item in tree()), "uncertain result was not attached to its outgoing item"
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == "Keep this newer draft", "late result replaced newer text"
    call("click_element", elementHandle=element("Edit", "Button")["handle"])
    call("click_element", elementHandle=element("Append", "Button")["handle"])
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
    demo_scenario("Photo preview")
    time.sleep(0.05)
    preview = next(item for item in tree() if item.get("accessibleLabel") == "Photo preview" and item.get("accessibleRole") == "Image")
    timestamp = next(item for item in tree() if item.get("accessibleRole") == "Text" and item.get("accessibleLabel", "").startswith("2026/10/15 12:05"))
    assert preview["size"]["height"] == 180, "photo exceeded reserved preview height"
    assert timestamp["absolutePosition"]["x"] >= preview["absolutePosition"]["x"] + preview["size"]["width"], "timestamp overlapped the photo instead of sitting beside it"
    assert timestamp["size"]["height"] <= 32, "timestamp exceeded a single line at the supported time type scale"
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
    demo_scenario("Sticker preview")
    time.sleep(0.05)
    element("[sticker]", "Text")
    before_sticker_time = element("2026/10/15 12:09", "Text")["absolutePosition"]
    time.sleep(1.1)
    element("Message image", "Image")
    assert not any(item.get("accessibleLabel") == "[sticker]" or item.get("accessibleValue") == "[sticker]" for item in tree()), "loaded sticker retained its fallback label"
    assert element("2026/10/15 12:09", "Text")["absolutePosition"] == before_sticker_time, "sticker completion moved its timestamp"
    print("PASS: sticker fallback disappears after loading without moving the timestamp")
    demo_scenario("Group preview")
    time.sleep(0.05)
    # Inspect both ends because the final timestamp can be clipped. Scope to
    # history: a browser relative time may also legitimately say "Yesterday".
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 600, "y": 280}, deltaY=2000, deltaX=0)
    time.sleep(0.05)
    grouped = tree()
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 600, "y": 280}, deltaY=-2000, deltaX=0)
    time.sleep(0.05)
    scrolled_group = tree()

    def history_rows(rows):
        history = next(item for item in rows if item.get("accessibleLabel") == "Message history" and item.get("accessibleRole") == "List")
        origin, size = history["absolutePosition"], history["size"]
        return [item for item in rows
                if "x" in item.get("absolutePosition", {}) and "y" in item.get("absolutePosition", {})
                and "height" in item.get("size", {})
                and item["absolutePosition"]["x"] >= origin["x"]
                and item["absolutePosition"]["x"] < origin["x"] + size["width"]
                and item["absolutePosition"]["y"] < origin["y"] + size["height"]
                and item["absolutePosition"]["y"] + item["size"]["height"] > origin["y"]]

    unique_group = {json.dumps(item["handle"], sort_keys=True): item
                    for rows in (grouped, scrolled_group) for item in history_rows(rows)}
    labels = [item.get("accessibleLabel", "") for item in unique_group.values()]
    assert labels.count("Sender: 葵") == 2 and labels.count("Sender: 蓮") == 1, "sender names were missing or repeated within a group"
    sender_avatars = [item for item in unique_group.values() if item.get("accessibleRole") == "Image" and item.get("accessibleLabel") in ("葵 avatar", "蓮 avatar")]
    assert len(sender_avatars) == 3 and all(item["size"] == {"width": 32.0, "height": 32.0} for item in sender_avatars), "sender avatar grouping or bounds failed"
    def day_label(year, month, day):
        date = datetime.date(year, month, day)
        age = (datetime.date.today() - date).days
        return "Today" if age == 0 else "Yesterday" if age == 1 else date.strftime("%A") + f" {month}/{day}" if 2 <= age <= 6 else f"{year}/{month}/{day}"
    assert labels.count(day_label(2026, 10, 15)) == 1 and labels.count(day_label(2026, 10, 16)) == 1, "calendar dates were repeated or omitted"
    times = set()
    for visible_group in (history_rows(grouped), history_rows(scrolled_group)):
        message_bodies = [item for item in visible_group if item.get("accessibleRole") == "TextInput" and any(base.get("id", "").endswith("::body") for base in item.get("typeNamesAndIds", []))]
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
    assert not any(item.get("accessibleLabel") == "Message draft" for item in tree()), "sheet exposed the covered composer to accessibility"
    call("click_element", elementHandle=element("Chats", "Button")["handle"])
    assert not any(item.get("accessibleLabel") == "Close profile" for item in tree()), "destination switch left profile open"
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    call("click_element", elementHandle=element("Open profile: 葵", "Button")["handle"])
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    element("Search loaded friends and groups", "TextInput")
    call("click_element", elementHandle=element("Chats", "Button")["handle"])
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == draft, "friend browsing lost draft"
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    assert call("get_element_properties", elementHandle=friend_search).get("accessibleValue") == "葵", "section switch lost friend query"
    call("set_element_value", elementHandle=friend_search, value="")
    call("click_element", elementHandle=element("Groups", "Button")["handle"])
    assert not any(item.get("accessibleLabel", "").startswith("Open profile:") and "Weekend plans" in item.get("accessibleLabel", "") for item in tree()), "collapsed groups remained visible"
    call("click_element", elementHandle=element("Groups", "Button")["handle"])
    call("click_element", elementHandle=element("Open profile: 新しい友だち 🌸", "Button")["handle"])
    call("click_element", elementHandle=element("Open chat", "Button")["handle"])
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue", "") == "", "new friend inherited another draft"
    element("No messages yet", "Text")
    assert element("Chats", "Button").get("accessibleItemSelected") is True, "Open chat left the rail on Friends"
    row = element("新しい友だち 🌸", "Button")
    assert row.get("accessibleItemSelected") is True, "Open chat left the chat row unselected"
    assert ", New, No messages yet" in row["accessibleLabel"], "new chat row lacks its empty preview/time label"
    for label in ("葵", "Weekend plans", "Notes to self"):
        element(label, "Button")
    assert row["absolutePosition"]["y"] > element("葵", "Button")["absolutePosition"]["y"], "Open chat scrolled away from the first rows"
    demo_scenario("Long chat list")
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    call("click_element", elementHandle=element("Open profile: 新しい友だち 🌸", "Button")["handle"])
    call("click_element", elementHandle=element("Open chat", "Button")["handle"])
    row = element("新しい友だち 🌸", "Button")
    viewport = element("Conversations", "List")
    assert row["absolutePosition"]["y"] >= viewport["absolutePosition"]["y"], "far row was not revealed"
    assert row["absolutePosition"]["y"] + row["size"]["height"] <= viewport["absolutePosition"]["y"] + viewport["size"]["height"] + 1, "far row remained clipped"
    assert abs(row["absolutePosition"]["y"] + row["size"]["height"] - viewport["absolutePosition"]["y"] - viewport["size"]["height"]) <= 1, "Open chat did not use the minimum scroll to reveal the far row"
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    call("set_element_value", elementHandle=friend_search, value="葵")
    call("click_element", elementHandle=element("Open profile: 葵", "Button")["handle"])
    call("click_element", elementHandle=element("Open chat", "Button")["handle"])
    call("click_element", elementHandle=element("葵", "Button")["handle"])
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == draft, "opening a new friend lost prior draft"
    print("PASS: friend search keyboard activation, profile Escape/destination closure, retained query/categories, Chats selection/empty state, new-chat isolation and prior draft")

if args.auth_recovery:
    before = call("get_element_properties", elementHandle=composer).get("accessibleValue")
    tracked = [("Find a conversation", "TextInput"), ("Conversations", "List"), ("Message history", "List"), ("Message draft", "TextInput")]
    positions = {label: element(label, role)["absolutePosition"].get("y", 0) for label, role in tracked}

    def no_strip():
        assert not any(item.get("accessibleLabel") in ("Connection needs attention", "Retry connection", "Sign in again") for item in tree()), "quiet state raised recovery controls"

    def unchanged_shell():
        current = {label: element(label, role)["absolutePosition"].get("y", 0) for label, role in tracked}
        assert current == positions, f"connection state moved shell content: {positions} -> {current}"

    def disjoint(left, right):
        a, b = left["absolutePosition"], right["absolutePosition"]
        s, t = left["size"], right["size"]
        return (a.get("x", 0) + s.get("width", 0) <= b.get("x", 0) or b.get("x", 0) + t.get("width", 0) <= a.get("x", 0)
                or a.get("y", 0) + s.get("height", 0) <= b.get("y", 0) or b.get("y", 0) + t.get("height", 0) <= a.get("y", 0))

    def safe_strip(label="Connection needs attention"):
        strip = element(label, "Banner")
        history = element("Message history", "List")
        assert strip["absolutePosition"].get("y", 0) == history["absolutePosition"].get("y", 0), "feedback is not below the conversation header"
        assert strip["size"]["height"] <= 52, "strip is no longer one line"
        for control, role in [("Chats", "Button"), ("Friends", "Button"), ("Settings", "Button"), ("Find a conversation", "TextInput"), ("Message draft", "TextInput"), ("Send", "Button")]:
            assert disjoint(strip, element(control, role)), f"feedback covered {control}"
        return strip

    demo_scenario("Reconnect blip")
    element("Connecting…", "Text")
    no_strip()
    unchanged_shell()
    deadline = time.monotonic() + 4
    while time.monotonic() < deadline:
        if any(item.get("accessibleLabel") == "Reconnected" for item in tree()): break
        time.sleep(.05)
    element("Reconnected", "Text")
    safe_strip("Connection restored")
    no_strip()
    unchanged_shell()
    time.sleep(3.2)
    assert not any(item.get("accessibleLabel") == "Reconnected" for item in tree()), "confirmation did not expire"
    demo_scenario("Offline")
    strip = safe_strip()
    element("Offline", "Text")
    assert not any(item.get("accessibleLabel") == "Sign in again" for item in tree()), "network-only failure offered sign-in"
    unchanged_shell()
    call("click_element", elementHandle=search)
    call("dispatch_key_event", windowHandle=window, text="\t")
    call("dispatch_key_event", windowHandle=window, text="\t")
    assert element("Retry connection", "Button").get("accessibleDescription") == "Focused", "forward Tab skipped recovery"
    call("dispatch_key_event", windowHandle=window, text="\uf709")  # F6 leaves for history
    call("dispatch_key_event", windowHandle=window, text="\uf709")  # F6 returns to recovery
    assert element("Retry connection", "Button").get("accessibleDescription") == "Focused"
    call("dispatch_key_event", windowHandle=window, text="\n")
    element("Retrying…", "Text")
    assert not element("Retry connection", "Button").get("accessibleEnabled"), "retry stayed enabled while pending"
    time.sleep(1.2)
    assert element("Retry connection", "Button").get("accessibleEnabled")
    call("click_element", elementHandle=composer)
    demo_scenario("Login recovery")  # The failed startup path, without a usable session.
    safe_strip()
    element("Signed out", "Text")
    element("Sign in again", "Button")
    unchanged_shell()
    call("click_element", elementHandle=composer)
    call("dispatch_key_event", windowHandle=window, text="\uf709")
    assert element("Sign in again", "Button").get("accessibleDescription") == "Focused"
    call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Press")
    call("dispatch_key_event", windowHandle=window, text="\t")
    call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Release")
    assert element("Retry connection", "Button").get("accessibleDescription") == "Focused"
    call("dispatch_key_event", windowHandle=window, text="\t")
    call("dispatch_key_event", windowHandle=window, text="\n")
    assert any("Chrome device slot" in item.get("accessibleLabel", "") for item in tree()), "sign-in did not explain its scope"
    assert any("encrypted drafts" in item.get("accessibleLabel", "") for item in tree()), "sign-in did not explain draft isolation"
    call("click_element", elementHandle=element("Start sign-in", "Button")["handle"])
    assert any("No pairing request was made" in item.get("accessibleLabel", "") for item in tree()), "offline sign-in guard did not report its result"
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    assert element("Sign in again", "Button").get("accessibleDescription") == "Focused"
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == before, "sign-in dismissal changed the draft"
    call("dispatch_key_event", windowHandle=window, text="\t")
    assert element("Message history", "List").get("accessibleDescription") == "Focused history", "Tab left recovery outside the reading order"
    demo_scenario("Expired sign-in")
    element("Session expired", "Text")
    call("click_element", elementHandle=element("Retry connection", "Button")["handle"])
    element("Retrying…", "Text")
    time.sleep(1.2)
    assert element("Sign in again", "Button").get("accessibleEnabled"), "failed expiry Retry lost sign-in"
    call("click_element", elementHandle=composer)
    demo_scenario("Reconnect blip")  # A successful local connection clears the unavailable-session state.
    # Expiry stays actionable until success; this fixture explicitly opens a usable session.
    time.sleep(2.2)
    demo_scenario("List load failure")
    no_strip()
    element("Couldn’t load conversations. Retry.", "Text")
    element("Retry conversations", "Button")
    print(f"PASS: quiet reconnect/no header overlap, transient expiry, network/startup/expired recovery, pending Retry, F6/Tab order, offline pairing guard, footer Retry; unchanged shell y={positions}; strip y={strip['absolutePosition'].get('y', 0)} height={strip['size']['height']}")

if args.layers:
    call("click_element", elementHandle=composer)
    for dismiss in ("escape", "close"):
        call("dispatch_key_event", windowHandle=window, text="\uf70f")
        assert element("Close demo scenarios", "Button").get("accessibleDescription") == "Focused"
        if dismiss == "escape":
            call("dispatch_key_event", windowHandle=window, text="\x1b")
        else:
            call("click_element", elementHandle=element("Close demo scenarios", "Button")["handle"])
        assert call("get_element_properties", elementHandle=composer).get("accessibleDescription") == "Focused message draft", "scenarios did not restore the composer focus"
    before = call("get_element_properties", elementHandle=composer).get("accessibleValue")
    call("click_element", elementHandle=element("Settings", "Button")["handle"])
    close_settings = element("Close settings", "Button")
    assert close_settings.get("absolutePosition", {}).get("x", 0) > 0, "wide Settings did not form a centered dialog"
    element("Enter sends", "Checkbox")
    assert call("get_element_properties", elementHandle=close_settings["handle"]).get("accessibleDescription") == "Focused", "Settings did not focus its close control"
    call("dispatch_key_event", windowHandle=window, text="\t")
    assert element("Settings section", "TabList").get("accessibleDescription") == "Focused", "Settings navigation did not describe focus"
    for _ in range(12):
        call("dispatch_key_event", windowHandle=window, text="\t")
    assert call("get_element_properties", elementHandle=composer).get("accessibleDescription") != "Focused message draft", "Settings let Tab escape to the draft"
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    assert element("Navigation", "Button").get("accessibleDescription") == "Focused navigation", "Settings did not restore rail focus"
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == before
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    friend_search = element("Search loaded friends and groups", "TextInput")["handle"]
    call("set_element_value", elementHandle=friend_search, value="葵")
    call("click_element", elementHandle=element("Open profile: 葵", "Button")["handle"])
    profile_close_y = element("Close profile", "Button")["absolutePosition"].get("y", 0)
    assert not any(item.get("accessibleLabel") == "Message draft" for item in tree()), "sheet exposed the covered conversation"
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    assert element("Friends and groups", "List").get("accessibleDescription") == "Focused friends and groups", "profile did not restore its list opener"
    call("click_element", elementHandle=element("Chats", "Button")["handle"])
    demo_scenario("Photo preview")
    call("click_element", elementHandle=element("Open photo", "Button")["handle"])
    call("click_element", elementHandle=element("Next", "Button")["handle"])
    assert any("800 × 1200" in item.get("accessibleLabel", "") for item in tree()), "Next left the loaded photo scope"
    call("click_element", elementHandle=element("Previous", "Button")["handle"])
    call("dispatch_key_event", windowHandle=window, text="1")
    image = element("Photo in viewer", "Image")
    assert image["size"]["width"] >= 1199, "1 did not show original size"
    pointer = {"x": 400, "y": 300}
    call("dispatch_pointer_scroll", windowHandle=window, position=pointer, deltaY=80, modifiers={"control": True})
    zoomed = element("Photo in viewer", "Image")
    assert zoomed["size"]["width"] > image["size"]["width"], "Ctrl+wheel did not zoom"
    fraction = (pointer["x"] - image["absolutePosition"]["x"]) / image["size"]["width"]
    assert abs(zoomed["absolutePosition"]["x"] + fraction * zoomed["size"]["width"] - pointer["x"]) < 3, "wheel zoom moved the point under the pointer"
    center = {"x": zoomed["absolutePosition"]["x"] + zoomed["size"]["width"] / 2 + 80, "y": 350}
    call("drag_element", elementHandle=zoomed["handle"], target=center)
    assert abs(element("Photo in viewer", "Image")["absolutePosition"]["x"] - zoomed["absolutePosition"]["x"]) > 20, "drag did not pan"
    call("dispatch_key_event", windowHandle=window, text="0")
    assert element("Photo in viewer", "Image")["size"]["width"] < 1199, "0 did not fit the photo"
    call("click_element", elementHandle=element("Save photo", "Button")["handle"])
    for _ in range(100):
        saved = [item.get("accessibleLabel", "") for item in tree() if item.get("accessibleLabel", "").startswith("Saved to ")]
        if saved: break
        time.sleep(.02)
    assert saved, "Save did not report a completed export"
    exported = Path(saved[0].removeprefix("Saved to "))
    assert exported.is_relative_to(Path("/tmp/line-desktop-demo-exports")), "preview export escaped its isolated folder"
    assert exported.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
    exported.unlink()
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    assert any(item.get("accessibleLabel") == "Open photo" and item.get("accessibleDescription") == "Focused" for item in tree()), "viewer did not restore the photo opener"
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == before, "layers changed the draft"
    demo_scenario("Login recovery")
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    call("click_element", elementHandle=element("Open profile: 葵", "Button")["handle"])
    close = element("Close profile", "Button")
    assert close["absolutePosition"].get("y", 0) == profile_close_y, "connection feedback moved the profile"
    assert not any(item.get("accessibleLabel") == "Connection needs attention" for item in tree()), "sheet did not cover the conversation feedback"
    browser_search = element("Search loaded friends and groups", "TextInput")
    assert browser_search["absolutePosition"]["x"] + browser_search["size"]["width"] <= close["absolutePosition"]["x"], "profile hid the browser"
    print("PASS: Settings modal focus, profile reading/focus/browser/connection-strip placement, viewer keys/wheel/pan/loaded-photo navigation, isolated PNG export, opener restoration, draft retention")

if args.reading:
    def reading_key(text):
        call("dispatch_key_event", windowHandle=window, text=text)

    def reading_bodies():
        return [item for item in tree() if item.get("accessibleRole") in ("TextInput", "Text") and item.get("accessibleLabel", "").startswith(("Incoming from ", "Outgoing, "))]

    demo_scenario("Reading fixtures")
    time.sleep(0.05)
    labels = [item.get("accessibleLabel", "") for item in tree()]
    # MCP exposes clipped rows only while visible; inspect both fixture halves.
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 650, "y": 280}, deltaY=-1800, deltaX=0)
    time.sleep(0.05)
    labels.extend(item.get("accessibleLabel", "") for item in tree())
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 650, "y": 280}, deltaY=1800, deltaX=0)
    time.sleep(0.05)
    for expected in ("Photo", "Sticker", "Video", "Voice message", "File", "Location", "Contact", "System message", "Message withdrawn", "Unable to decrypt message", "Unknown message"):
        assert any(expected in label for label in labels), f"missing placeholder: {expected}"
    first = next(item for item in reading_bodies() if "https://example.org/reading" in item.get("accessibleValue", ""))
    first_y = first["absolutePosition"]["y"]
    demo_scenario("History load")
    time.sleep(0.05)
    element("Loading recent messages…", "Text")
    assert call("get_element_properties", elementHandle=first["handle"])["absolutePosition"]["y"] == first_y, "loading strip shifted history"
    demo_scenario("History load")
    assert call("get_element_properties", elementHandle=first["handle"])["absolutePosition"]["y"] == first_y, "loading completion shifted history"
    url = "https://example.org/reading"
    link = element("Open link " + url, "Button")
    assert link.get("accessibleDescription") == url, "full link cannot be inspected"
    call("invoke_accessibility_action", elementHandle=link["handle"], action="Default_")
    assert element("Recorded link activation", "Text").get("accessibleValue") == url, "demo did not record link without external browser"
    before_copy = call("get_element_properties", elementHandle=composer).get("accessibleValue", "")
    other_link = element("Open link https://www.example.jp", "Button")
    target = other_link["absolutePosition"]
    call("drag_element", elementHandle=other_link["handle"], target={"x": target["x"] + 2, "y": target["y"] + 2})
    assert element("Recorded link activation", "Text").get("accessibleValue") == url, "drag activated a link"
    call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Press")
    reading_key("c")
    call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Release")
    call("set_element_value", elementHandle=composer, value="")
    call("click_element", elementHandle=composer)
    call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Press")
    reading_key("v")
    call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Release")
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == first.get("accessibleValue"), "dragging URL action did not select original message text"
    call("set_element_value", elementHandle=composer, value=before_copy)
    call("click_element", elementHandle=element("Show message", "Button")["handle"])
    first = next(item for item in reading_bodies() if "https://example.org/reading" in item.get("accessibleValue", ""))
    link = element("Open link " + url, "Button")
    call("click_element", elementHandle=link["handle"], button="Right")
    copy_link = next(item for item in tree() if item.get("accessibleLabel") == "Copy link")
    call("click_element", elementHandle=copy_link["handle"])
    call("set_element_value", elementHandle=composer, value="")
    call("click_element", elementHandle=composer)
    call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Press")
    reading_key("v")
    call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Release")
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == url, "Copy link did not copy normalized address"
    call("set_element_value", elementHandle=composer, value=before_copy)

    call("click_element", elementHandle=search)
    tabs = None
    for count in range(1, 9):
        reading_key("\t")
        if call("get_element_properties", elementHandle=composer).get("accessibleDescription") == "Focused message draft":
            tabs = count
            break
    assert tabs is not None, "search did not reach composer in eight Tabs"
    # Reverse traversal reaches the single history surface, regardless of rows.
    for _ in range(2):
        call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Press")
        call("dispatch_key_event", windowHandle=window, text="\t")
        call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Release")
        history = element("Message history", "List")
        if history.get("accessibleDescription") == "Focused history":
            break
    assert history.get("accessibleDescription") == "Focused history", "reverse Tab entered a message"
    reading_key("\uf72b")  # End
    bottom = call("get_element_properties", elementHandle=first["handle"])["absolutePosition"]["y"]
    reading_key("\uf729")  # Home
    top = call("get_element_properties", elementHandle=first["handle"])["absolutePosition"]["y"]
    assert top > bottom, "Home did not return to history beginning"
    reading_key("\uf701")  # Down
    down = call("get_element_properties", elementHandle=first["handle"])["absolutePosition"]["y"]
    assert down < top, "Down did not scroll history"
    reading_key("\uf700")  # Up
    up = call("get_element_properties", elementHandle=first["handle"])["absolutePosition"]["y"]
    assert up > down, "Up did not scroll history"
    reading_key("\uf72b")  # End
    reading_key("\uf72c")  # PageUp
    element("Latest", "Button")
    reading_key("\uf72d")  # PageDown
    reading_key("\uf72b")  # End
    time.sleep(0.05)
    last = next(item for item in reading_bodies() if "Unknown message" in item.get("accessibleValue", ""))
    call("click_element", elementHandle=last["handle"])
    assert not any(item.get("accessibleLabel") == "Latest" and item.get("accessibleRole") == "Button" for item in tree()), "click focus disabled following"
    reading_key("\uf700")  # Up from a click-focused body scrolls on the first press.
    element("Latest", "Button")
    call("click_element", elementHandle=last["handle"])
    # Selection release can be emitted synchronously while a chat model is
    # replaced; it must queue deferred work rather than borrow View reentrantly.
    call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Press")
    reading_key("a")
    call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Release")
    call("click_element", elementHandle=element("Weekend plans", "Button")["handle"])
    call("click_element", elementHandle=element("葵", "Button")["handle"])
    assert element("Message draft", "TextInput").get("accessibleValue") == before_copy, "selected-body chat switch lost draft or failed"
    demo_scenario("Reading fixtures")
    print(f"PASS: reading search→composer in {tabs} Tabs, single history focus/scroll, stable loading, demo link activation and 12 content kinds")

if args.rich:
    demo_scenario("Rich content")
    time.sleep(0.3)
    def rich_tree():
        result = call("get_element_tree", elementHandle=root, maxElements=2500)
        assert not result.get("truncated"), "rich fixture exceeded bounded tree"
        return result["elements"]
    def rich_element(label, role):
        return next(item for item in rich_tree() if item.get("accessibleLabel") == label and item.get("accessibleRole") == role)
    emoji_image = rich_element("[笑]", "Image")
    rich_body = next(item for item in rich_tree() if item.get("accessibleRole") == "Text" and "日😀[笑]本🌸" in item.get("accessibleLabel", ""))
    assert rich_body["size"]["height"] > emoji_image["size"]["height"] * 2, "rich text did not wrap into measured lines"
    assert emoji_image["absolutePosition"]["x"] > rich_body["absolutePosition"]["x"], "inline image overlapped the first text run"
    labels = [item.get("accessibleLabel", "") for item in rich_tree()]
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 800, "y": 400}, deltaY=-800, deltaX=0)
    time.sleep(0.05)
    labels += [item.get("accessibleLabel", "") for item in rich_tree()]
    for expected in ("2.0 KB", "2:05", "0:12", "Contact · 葵", "葵 joined the group", "Message withdrawn"):
        assert any(expected in label for label in labels), f"missing rich metadata: {expected}"
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 800, "y": 400}, deltaY=1600, deltaX=0)
    time.sleep(0.05)
    demo_scenario("Offline")
    reply = next(item for item in rich_tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel") == "Jump to replied message")
    def emoji_body():
        return next(item for item in rich_tree() if item.get("accessibleRole") == "Text" and "日😀[笑]本🌸" in item.get("accessibleLabel", ""))
    origin_y = emoji_body()["absolutePosition"]["y"]
    call("click_element", elementHandle=reply["handle"])
    time.sleep(0.05)
    rich_element("Return to message", "Button")
    latest = next(item for item in rich_tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith("Latest"))
    return_pill = rich_element("Return to message", "Button")
    assert abs(latest["absolutePosition"]["y"] - return_pill["absolutePosition"]["y"]) <= 1, "history pills stacked vertically"
    assert latest["absolutePosition"]["x"] + latest["size"]["width"] <= return_pill["absolutePosition"]["x"], "history pills overlap"
    strip = rich_element("Connection needs attention", "Banner")
    history = rich_element("Message history", "List")
    editor = rich_element("Message draft", "TextInput")
    assert strip["absolutePosition"]["y"] == history["absolutePosition"]["y"], "strip left the history top"
    for pill in (latest, return_pill):
        assert strip["absolutePosition"]["y"] + strip["size"]["height"] <= pill["absolutePosition"]["y"], "strip covered a history pill"
        assert pill["absolutePosition"]["y"] + pill["size"]["height"] <= editor["absolutePosition"]["y"], "history pill covered the composer"
    call("click_element", elementHandle=composer)
    for _ in range(3):
        call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Press")
        call("dispatch_key_event", windowHandle=window, text="\t")
        call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Release")
    assert rich_element("Message history", "List").get("accessibleDescription") == "Focused history", "reverse Tab skipped a history pill"
    for _ in range(3):
        call("dispatch_key_event", windowHandle=window, text="\t")
    assert call("get_element_properties", elementHandle=composer).get("accessibleDescription") == "Focused message draft", "history/Latest/Return/composer Tab order changed"
    call("click_element", elementHandle=rich_element("Return to message", "Button")["handle"])
    time.sleep(0.05)
    assert abs(emoji_body()["absolutePosition"]["y"] - origin_y) <= 1, "reply return lost the reading anchor"
    call("click_element", elementHandle=emoji_body()["handle"])
    rich_element("[笑]", "Image")
    assert not any(item.get("accessibleLabel") == "Show emoji" for item in rich_tree()), "click entered emoji fallback mode"
    assert emoji_body().get("accessibleDescription") == "Focused message", "click did not focus the bubble"
    call("click_element", elementHandle=emoji_body()["handle"], button="Right")
    select_text = next(item for item in rich_tree() if item.get("accessibleLabel") == "Select text")
    call("click_element", elementHandle=select_text["handle"])
    body = next(item for item in rich_tree() if item.get("accessibleRole") == "TextInput" and "日😀[笑]本🌸" in item.get("accessibleValue", ""))
    original = body["accessibleValue"]
    def rich_ctrl(letter):
        call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Press")
        call("dispatch_key_event", windowHandle=window, text=letter)
        call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Release")
    call("click_element", elementHandle=body["handle"])
    rich_ctrl("a")
    rich_ctrl("c")
    call("set_element_value", elementHandle=composer, value="")
    call("click_element", elementHandle=composer)
    rich_ctrl("v")
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == original, "rich copy lost emoji fallback or newlines"
    call("click_element", elementHandle=rich_element("Show emoji", "Button")["handle"])
    rich_element("[笑]", "Image")
    call("click_element", elementHandle=rich_element("Save", "Button")["handle"])
    time.sleep(0.2)
    assert any(item.get("accessibleLabel") == "Saved" for item in rich_tree()), "synthetic file Save did not finish"
    assert any(path.read_bytes() == b"Synthetic attachment\n" for path in Path("/tmp/line-desktop-demo-exports").glob("*") if path.is_file()), "demo export bytes differ"
    print("PASS: inline rich image, fallback/newline clipboard, reply jump/return, file duration metadata and synthetic Save")

if args.screenshot:
    result = rpc("tools/call", {"name": "take_screenshot", "arguments": {"windowHandle": window}})
    image = next(item for item in result["content"] if item["type"] == "image")
    args.screenshot.write_bytes(base64.b64decode(image["data"]))
print(f"PASS: MCP draft retention, chat selection, filtering, disabled offline Send, and multiline input; scale {properties.get('scaleFactor')}")

if args.paging:
    import re
    def rss_kib():
        if not args.paging_pid: return None
        match = re.search(r"^VmRSS:\s+(\d+)", Path(f"/proc/{args.paging_pid}/status").read_text(), re.M)
        return int(match.group(1)) if match else None
    def paging_bodies():
        return [item for item in tree() if item.get("accessibleRole") == "TextInput" and item.get("accessibleValue", "").startswith("History item ")]
    def state_counts():
        call("dispatch_key_event", windowHandle=window, text="\uf70f")  # F12
        text = next(item["accessibleLabel"] for item in tree() if item.get("accessibleLabel", "").startswith("History window "))
        call("click_element", elementHandle=element("Close demo scenarios", "Button")["handle"])
        window_count, retained_count, page_count = map(int, re.findall(r"\d+", text))
        assert window_count <= 200 and retained_count <= 600, "history exceeded the UI or retained-row budget"
        return page_count
    demo_scenario("Paged history")
    time.sleep(.15)
    rss_before = rss_kib()
    failure_seen = False
    anchor_checks = 0
    for step in range(85):
        call("dispatch_pointer_scroll", windowHandle=window, position={"x": 600, "y": 280}, deltaY=100000, deltaX=0)
        time.sleep(.015)
        candidates = [body for body in paging_bodies() if 100 <= body["absolutePosition"]["y"] < 400]
        anchor = candidates[0] if candidates else None
        time.sleep(.15)
        page_count = state_counts()
        if anchor:
            same = [body for body in paging_bodies() if body.get("accessibleValue") == anchor.get("accessibleValue")]
            if same:
                assert same[0]["absolutePosition"]["y"] == anchor["absolutePosition"]["y"], "older load/failure moved the anchored message"
                anchor_checks += 1
        retries = [item for item in tree() if item.get("accessibleLabel") == "Retry older" and item.get("accessibleRole") == "Button"]
        if retries:
            failure_seen = True
            before_retry = paging_bodies()
            call("click_element", elementHandle=retries[0]["handle"])
            time.sleep(.15)
            after_retry = paging_bodies()
            for old in before_retry:
                same = [body for body in after_retry if body.get("accessibleValue") == old.get("accessibleValue")]
                if same: assert same[0]["absolutePosition"]["y"] == old["absolutePosition"]["y"], "retry moved the reading anchor"
        if any(item.get("accessibleLabel") == "Beginning of conversation" for item in tree()):
            break
    else: raise AssertionError("older paging never reached the beginning")
    assert failure_seen and anchor_checks >= 3, "failure/retry or three prepend anchors were not exercised"
    assert state_counts() >= 20, "did not traverse the thousand-message fixture"
    first = [body for body in paging_bodies() if body.get("accessibleValue", "").startswith("History item 0000")]
    assert first, "beginning did not expose the first message"
    before_end = state_counts()
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 600, "y": 280}, deltaY=100000, deltaX=0)
    time.sleep(.2)
    assert state_counts() == before_end, "beginning continued loading"
    rss_after = rss_kib()
    print(f"PASS: SC4 thousand-message paging, failure/retry/end, {anchor_checks} stable prepend anchors, UI<=200/retained<=600; RSS KiB before={rss_before} after={rss_after}")

if args.unread:
    demo_scenario("Long history")
    time.sleep(.1)
    call("dispatch_pointer_scroll",windowHandle=window,position={"x":600,"y":280},deltaY=700,deltaX=0)
    time.sleep(.1)
    candidates=[item for item in tree() if item.get("accessibleRole")=="TextInput" and any(base.get("id","").endswith("::body") for base in item.get("typeNamesAndIds",[])) and 120<item["absolutePosition"]["y"]<380]
    anchor=candidates[0] if candidates else None
    demo_scenario("Unread scenario")
    time.sleep(.25)
    current=element("葵","Button")
    other=element("Weekend plans","Button")
    assert ", 3 unread" in current["accessibleLabel"],"open-chat arrivals lost unread before exposure"
    assert ", 2 unread" in other["accessibleLabel"],"other-chat unread badge did not update"
    if anchor:
        after=call("get_element_properties",elementHandle=anchor["handle"])
        assert after["absolutePosition"]["y"]==anchor["absolutePosition"]["y"],"unread arrivals moved the reading anchor"
    element("Latest · 3 new","Button")
    # The divider can be outside the bounded rendered range while scrolled up.
    call("click_element",elementHandle=element("Latest · 3 new","Button")["handle"])
    time.sleep(.25)
    # A background automation window must stay unread. An already-active owner
    # may expose and read all three; focus is never requested by this test.
    current=element("葵","Button")
    assert not any(f", {n} unread" in current["accessibleLabel"] for n in [1,2]),"complete exposure left a partial boundary"
    element("Unread messages","Text")
    assert ", 2 unread" in element("Weekend plans","Button")["accessibleLabel"],"reading one chat cleared another chat"
    print("PASS: unread row badges, separate-chat counts, persistent divider and stable scrolled arrival anchor; no native focus request")
