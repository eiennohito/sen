#!/usr/bin/env python3
"""Exercise an already-running --demo window through Slint's optional MCP server.

Agents must launch their owned preview with tools/run-ui-debug.py; do not map
a debug window on the user's desktop. Pass that scope's unique --port here.
"""
import argparse
import base64
import json
import time
import tempfile
import datetime
from pathlib import Path
import urllib.request

from mcp_preview import add_preview_arguments, launch_preview

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--port", type=int, default=9447)
parser.add_argument("--screenshot", type=Path)
parser.add_argument("--unread", action="store_true", help="Check synthetic unread badges, divider and arrivals while reading older history")
parser.add_argument("--scenarios", action="store_true", help="Run synthetic arrival, media, clipboard, and uncertain-send scenarios")
parser.add_argument("--friends", action="store_true", help="Exercise offline friend search, profiles, categories and opening chats")
parser.add_argument("--layout-photos", action="store_true", help="Check bounded avatars/media, settings placement, and photo viewer")
parser.add_argument("--reading", action="store_true", help="Check reading focus, keyboard scroll, stable loading, links and content placeholders")
parser.add_argument("--rich", action="store_true", help="Check rich emoji fallback copy, reply jump/return, durations and file Save")
parser.add_argument("--media", action="store_true", help="Check each media failure reason and its item Retry")
parser.add_argument("--signed-out", action="store_true", help="Check a no-session account, QR delivery and cancellation")
parser.add_argument("--missing-module", action="store_true", help="With --signed-out, check a backend that cannot open its crypto module")
parser.add_argument("--auth-recovery", action="store_true", help="Check offline feedback, sign-in focus, synthetic sign-in failure and draft preservation")
parser.add_argument("--layers", action="store_true", help="Check dialogs, profile panels, viewer input, export, and opener focus")
parser.add_argument("--focus", action="store_true", help="Check every forward/reverse Tab stop on chats, friends and conversation, with and without recovery")
parser.add_argument("--compose", action="store_true", help="Check reactions, stickers and new chats")
parser.add_argument("--color-scheme", choices=("light", "dark"), default="light")
parser.add_argument("--surface-shots", type=Path, help="Write compose surface screenshots")
parser.add_argument("--wide", action="store_true", help="Check the 1280×900 side-panel layout")
add_preview_arguments(parser)
args = parser.parse_args()
if args.wide and args.launch:
    args.demo_size = "1280x900"
launch_preview(args)
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
    parsed = json.loads(result["content"][0]["text"])
    if name == "get_element_tree":
        assert not any("Internal error:" in str(item.get(field, "")) for item in parsed["elements"] for field in ("accessibleLabel", "accessibleValue", "accessibleDescription")), "debug internal-error toast appeared"
    return parsed

rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "line-ui-check", "version": "1"}})
window = call("list_windows")["windowHandles"][0]
properties = call("get_window_properties", windowHandle=window)
root = properties["rootElementHandle"]
if args.wide:
    scale = properties.get("scaleFactor", 1)
    assert (properties["size"]["width"] / scale, properties["size"]["height"] / scale) == (1280, 900), "--wide requires a 1280×900 window"

def tree():
    result = call("get_element_tree", elementHandle=root, maxElements=1500)
    assert not result.get("truncated"), "UI tree exceeded test bound"
    return result["elements"]

def element(label, role, read_tree=tree):
    # The engine applies commands on its worker, so a control may appear a moment after the click.
    deadline = time.monotonic() + 2
    while True:
        try:
            return next(item for item in read_tree() if (item.get("accessibleLabel") == label or item.get("accessibleLabel", "").startswith(label + ", ")) and item.get("accessibleRole") == role)
        except StopIteration:
            if time.monotonic() > deadline:
                labels=[(item.get("accessibleRole"),item.get("accessibleLabel"),item.get("size")) for item in read_tree() if item.get("accessibleLabel")]
                raise AssertionError(f"Missing {role} {label!r}; visible controls: {labels}") from None
            time.sleep(0.05)

def wait_for(predicate, message):
    # MCP input dispatch completes before worker deltas and layout callbacks.
    # Poll the observable outcome instead of assuming one RPC or a sleep drains them.
    deadline = time.monotonic() + 5
    while True:
        result = predicate()
        if result:
            return result
        assert time.monotonic() < deadline, message
        time.sleep(.05)


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
    started = datetime.datetime.now()
    call("dispatch_key_event", windowHandle=window, text="\uf70f")  # F12
    if label == "Load media":
        call("click_element", elementHandle=scenario_action("Media failures")["handle"])
    call("click_element", elementHandle=scenario_action(label)["handle"])
    scenario_bounds = (started, datetime.datetime.now())


def fixture_times(clock):
    hour, minute = map(int, clock.split(":"))
    age = (12 * 60 + 12 - hour * 60 - minute) * 60000
    start, end = scenario_bounds
    labels = set()
    for offset in range(-1, int((end - start).total_seconds()) + 3):
        now = start + datetime.timedelta(seconds=offset)
        elapsed = (now - now.replace(hour=0, minute=0, second=0, microsecond=0)).total_seconds() * 1000
        timestamp = now - datetime.timedelta(milliseconds=age * min(elapsed, 174 * 60000) / (174 * 60000))
        labels.add(timestamp.strftime("%Y/%m/%d %H:%M"))
    return labels

def show_chats():
    if properties["size"]["width"] / properties.get("scaleFactor", 1) < 740:
        back = next((item for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel") == "Back"), None)
        if back: call("click_element", elementHandle=back["handle"])
    call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Press")
    call("dispatch_key_event", windowHandle=window, text="2")
    call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Release")


if args.signed_out:
    def shot(name):
        if args.surface_shots:
            args.surface_shots.mkdir(parents=True, exist_ok=True)
            result = rpc("tools/call", {"name": "take_screenshot", "arguments": {"windowHandle": window}})
            image = next(item for item in result["content"] if item["type"] == "image")
            (args.surface_shots / f"{name}.png").write_bytes(base64.b64decode(image["data"]))

    def click(label):
        call("click_element", elementHandle=element(label, "Button")["handle"])

    # Verify the compiled demo marker before interacting with authentication.
    call("dispatch_key_event", windowHandle=window, text="\uf70f")
    marker = element("Synthetic interaction scenarios", "Text")
    assert any(part.get("id", "").endswith("::demo-marker") for part in marker.get("typeNamesAndIds", []))
    click("Close demo scenarios")
    if args.missing_module:
        click("Sign in")
        message = "Cannot read the LINE crypto module at /demo/crypto.wasm. Check LINE for Chrome or the developer override."
        element(message, "Text")
        element("Sign-in needs a supported LINE for Chrome installation.", "Text")
        for _ in range(2):
            click("Check again")
            element(message, "Text")
            element("Check again", "Button")
            assert not any(item.get("accessibleRole") == "Image" and "QR code" in item.get("accessibleLabel", "") for item in tree())
        click("Back")
        element("Not signed in", "Text")
        print("PASS: missing crypto module reported at startup and after repeated sign-in attempts")
        raise SystemExit(0)
    for section in ("Chats", "Friends"):
        click(section)
        element("Not signed in", "Text")
        assert not any(item.get("accessibleLabel") in ("Your profile", "Your avatar", "No friends yet", "No conversations yet", "Choose a conversation", "Message draft") for item in tree())
        shot(section.lower())
        click("Sign in")
        shot(section.lower() + "-before-start")
        assert not any(item.get("accessibleRole") == "Image" and "QR code" in item.get("accessibleLabel", "") for item in tree())
        click("Start")
        qr = element("Scan this QR code with LINE on your phone", "Image")
        assert qr["size"]["width"] == 220 and qr["size"]["height"] == 220
        time.sleep(.2)
        shot(section.lower() + "-qr")
        click("Back")
        click("Cancel sign-in")
        wait_for(lambda: any(item.get("accessibleLabel") == "Not signed in" for item in tree()), "Back did not cancel pairing")
    click("Chats")
    call("dispatch_key_event", windowHandle=window, text="\uf70f")
    element("Synthetic interaction scenarios", "Text")
    call("click_element", elementHandle=scenario_action("Sign-in keyring failure")["handle"])
    click("Sign in")
    click("Start")
    element("Unlock your keyring (KWallet/GNOME Keyring) and try again. Ensure Secret Service is running with a default keyring.", "Text")
    assert not any("Local data could not be saved" in item.get("accessibleLabel", "") for item in tree())
    click("Back")
    click("Settings")
    shot("settings")
    for section in ("Notifications", "Background"):
        call("click_element", elementHandle=element(section, "Tab")["handle"])
        shot("settings-" + section.lower())
    call("click_element", elementHandle=element("About", "Tab")["handle"])
    assert not any(item.get("accessibleRole") == "Button" and item.get("accessibleLabel") in ("", "Offline preview") for item in tree())
    shot("settings-about")
    print("PASS: signed-out Chats/Friends/rail/Settings, QR after Start, Back cancellation", flush=True)
    if args.launch:
        import subprocess
        import sys
        subprocess.run([sys.executable, __file__, "--launch", "--profile", args.profile,
                        "--port", str(args.port + 1), "--signed-out", "--missing-module"], check=True)
    raise SystemExit(0)

# Refuse to operate on a real account. Sends remain local to the demo transport.
call("dispatch_key_event", windowHandle=window, text="\uf70f")  # F12
demo_marker = element("Synthetic interaction scenarios", "Text")
assert any(part.get("id", "").endswith("::demo-marker") for part in demo_marker.get("typeNamesAndIds", [])), "refusing a window without the compiled demo-only marker"
call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Press")
call("dispatch_key_event", windowHandle=window, text="\t", eventType="PressAndRelease")
call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Release")
try:
    last_scenario = scenario_element("Demo exposed")
except StopIteration:
    last_scenario = scenario_element("Offline")
last_scenario_label = last_scenario["accessibleLabel"]
assert last_scenario.get("accessibleDescription") == "Focused", "reverse Tab did not focus the final scenario"
call("dispatch_key_event", windowHandle=window, text="\t", eventType="PressAndRelease")
assert element("Close demo scenarios", "Button").get("accessibleDescription") == "Focused", "Tab did not wrap from the last scenario to Close"
call("click_element", elementHandle=element("Close demo scenarios", "Button")["handle"])
if args.focus:
    def focus_key(text, event_type="PressAndRelease"):
        call("dispatch_key_event", windowHandle=window, text=text, eventType=event_type)

    def focus_label():
        focused = [item for item in tree() if item.get("accessibleDescription", "").startswith("Focused")]
        assert len(focused) == 1, f"unlabelled or ambiguous Tab stop: {focused}"
        label = focused[0].get("accessibleLabel", "")
        assert label, "focused control has no label"
        assert focused[0]["size"]["width"] > 0 and focused[0]["size"]["height"] > 0, f"hidden focus: {label}"
        return label

    def focus_tab(reverse=False):
        if reverse: focus_key("\x10", "Press")
        focus_key("\t")
        if reverse: focus_key("\x10", "Release")

    def check_chain(surface, expected, seed):
        assert len(expected) == len(set(expected)), f"{surface}: duplicate expected stop"
        if seed[1] == "TextInput":
            call("click_element", elementHandle=element(*seed)["handle"])
        for _ in expected:
            if focus_label() == seed[0]: break
            focus_tab()
        assert focus_label() == seed[0], f"{surface}: seed did not focus {seed[0]} (at {focus_label()})"
        # Every stop is a seed; each direction must visit exactly one full cycle.
        for _ in expected:
            label = focus_label()
            index = expected.index(label)
            for reverse in (False, True):
                seen = [label]
                for offset in range(1, len(expected) + 1):
                    focus_tab(reverse)
                    current = focus_label()
                    wanted = expected[(index + (-offset if reverse else offset)) % len(expected)]
                    assert current == wanted, f"{surface}: {'reverse' if reverse else 'forward'} from {label}: {seen} → {current}, expected {wanted}"
                    if offset < len(expected): seen.append(current)
                assert len(set(seen)) == len(expected) and set(seen) == set(expected), f"{surface}: incomplete cycle: {seen}"
                assert focus_label() == label, f"{surface}: did not return to seed {label}"
            focus_tab()
        print(f"PASS: {surface}: {' → '.join(expected)} → {expected[0]}; reverse mirrors forward, every seed")

    logical_width = properties["size"]["width"] / properties.get("scaleFactor", 1)
    narrow = logical_width < 740
    panel = logical_width >= 1060
    layer_kind = "panel" if panel else "sheet"
    def click_focus(label, role="Button"):
        call("click_element", elementHandle=element(label, role)["handle"])

    def browser():
        if narrow:
            focus_key("\x12", "Press")  # Alt+Left also works from layer openers.
            focus_key("\uf702")
            focus_key("\x12", "Release")

    def browser_stops(search_label, list_label, recovery):
        return ["Navigation", search_label] + (["All", "New chat"] if list_label == "Conversations" else []) + (recovery if narrow else []) + [list_label]

    # Chips are one Tab stop; arrows move focus without activating a filter.
    click_focus("Chats")
    click_focus("Find a conversation", "TextInput")
    focus_tab()
    assert focus_label() == "All"
    focus_key("\uf703")
    assert focus_label() == "Friends"
    focus_key(" ")
    assert focus_label() == "Friends"
    assert not any(item.get("accessibleLabel", "").startswith("Weekend plans, ") for item in tree()), "Space did not activate Friends filter"
    focus_key("\uf703")
    assert focus_label() == "Groups"
    focus_key("\n")
    element("Weekend plans", "Button")
    assert not any(item.get("accessibleLabel", "").startswith("葵, ") for item in tree()), "Enter did not activate Groups filter"
    focus_key("\uf702")
    focus_key("\uf702")
    focus_key("\n")
    assert focus_label() == "All"
    # Category headers are arrow-reachable list rows, with both activation keys.
    click_focus("Friends")
    click_focus("Search friends and groups", "TextInput")
    focus_tab()
    assert focus_label() == "Friends and groups"
    focus_key("\uf729")  # Home
    focus_key(" ")
    assert not any(item.get("accessibleLabel", "").startswith("Open profile: Weekend plans") for item in tree()), "Space did not collapse Groups"
    focus_key("\uf701")  # Next row is now the Friends category.
    focus_key("\n")
    assert not any(item.get("accessibleLabel", "").startswith("Open profile:") for item in tree()), "Enter did not collapse Friends"
    focus_key(" ")
    focus_key("\uf700")
    focus_key("\n")
    element("Open profile: 葵", "Button")
    element("Open profile: Weekend plans", "Button")
    print("PASS: filter chip Tab/arrows/Space/Enter and friend category row arrows/Space/Enter")
    for offline in (False, True):
        click_focus("Chats")
        click_focus("葵")
        browser()
        if offline: demo_scenario("Expired sign-in")
        suffix = "with strip" if offline else "without strip"
        recovery = ["Sign in again"] if offline else []
        for surface, search_label, list_label in [("chats", "Find a conversation", "Conversations"), ("friends", "Search friends and groups", "Friends and groups")]:
            click_focus("Friends" if surface == "friends" else "Chats")
            expected = browser_stops(search_label, list_label, recovery)
            if not narrow:
                expected += ["葵", "More"] + recovery + ["Message history", "Message draft"]
                if element("Stickers", "Button").get("accessibleEnabled"): expected.append("Stickers")
                if element("Attach", "Button").get("accessibleEnabled"): expected.append("Attach")
                if element("Send", "Button").get("accessibleEnabled"): expected.append("Send")
            check_chain(f"{surface} {suffix}", expected, (search_label, "TextInput"))
            call("set_element_value", elementHandle=element(search_label, "TextInput")["handle"], value="葵")
            check_chain(f"{surface} search clear {suffix}", expected[:2] + ["Clear search"] + expected[2:], (search_label, "TextInput"))
            click_focus("Clear search")
        click_focus("Chats")
        for chat in ("葵", "Weekend plans"):
            click_focus(chat)
            call("set_element_value", elementHandle=element("Message draft", "TextInput")["handle"], value="Focus check")
            expected = ["Back"] if narrow else ["Navigation", "Find a conversation", "All", "New chat", "Conversations"]
            expected.append(chat)
            if chat == "Weekend plans": expected.append(element("Members", "Button")["accessibleLabel"])
            expected += ["More"] + recovery + ["Message history", "Message draft"]
            if element("Stickers", "Button").get("accessibleEnabled"): expected.append("Stickers")
            if element("Attach", "Button").get("accessibleEnabled"): expected.append("Attach")
            if element("Send", "Button").get("accessibleEnabled"): expected.append("Send")
            check_chain(f"conversation {chat} {suffix}", expected, ("Message draft", "TextInput"))
            if chat == "Weekend plans":
                click_focus("Members")
                layer = ["Close details", "Search members", "Group members"]
                prefix = expected if panel else [] if narrow else ["Navigation", "Find a conversation", "All", "New chat", "Conversations"]
                layer_recovery = []
                check_chain(f"members {layer_kind} {suffix}", prefix + layer_recovery + layer, ("Close details", "Button"))
                call("set_element_value", elementHandle=element("Search members", "TextInput")["handle"], value="葵")
                check_chain(f"members search clear {suffix}", prefix + layer_recovery + layer[:2] + ["Clear search"] + layer[2:], ("Close details", "Button"))
                click_focus("Open member: 葵")
                check_chain(f"member profile {layer_kind} {suffix}", prefix + layer_recovery + ["Back", "Profile more", "Close details", "Open profile photo", "Chat", "Rename"], ("Close details", "Button"))
                focus_key("\x1b")
                call("set_element_value", elementHandle=element("Search members", "TextInput")["handle"], value="蓮")
                click_focus("Open member: 蓮")
                check_chain(f"member profile without chat {layer_kind} {suffix}", prefix + layer_recovery + ["Back", "Profile more", "Close details", "Open profile photo", "Add friend"], ("Close details", "Button"))
                focus_key("\x1b")
                focus_key("\x1b")
            browser()
        click_focus("Friends")
        click_focus("Open profile: 葵")
        prefix = [] if narrow else ["Navigation", "Search friends and groups", "Friends and groups"]
        if panel:
            prefix += ["Weekend plans", element("Members", "Button")["accessibleLabel"], "More"] + recovery + ["Message history", "Message draft"]
            if element("Stickers", "Button").get("accessibleEnabled"): prefix.append("Stickers")
            if element("Attach", "Button").get("accessibleEnabled"): prefix.append("Attach")
            if element("Send", "Button").get("accessibleEnabled"): prefix.append("Send")
        check_chain(f"profile {layer_kind} {suffix}", prefix + ["Profile more", "Close details", "Open profile photo", "Chat", "Rename"], ("Close details", "Button"))
        focus_key("\x1b")
        click_focus("Settings")
        check_chain(f"settings {suffix}", ["Close settings", "Settings section", "Message text size", "Enter sends"], ("Close settings", "Button"))
        focus_key("\x1b")
        click_focus("Chats")
    demo_scenario("Reconnect blip")
    time.sleep(2.2)
    # Leave a connected Chats browser with empty drafts and queries for later checks.
    for chat in ("Weekend plans", "葵"):
        show_chats()
        click_focus(chat)
        call("set_element_value", elementHandle=element("Message draft", "TextInput")["handle"], value="")
        if chat == "Weekend plans":
            click_focus("Members")
            call("set_element_value", elementHandle=element("Search members", "TextInput")["handle"], value="")
            focus_key("\x1b")
    show_chats()
    call("set_element_value", elementHandle=element("Find a conversation", "TextInput")["handle"], value="")
    deadline = time.monotonic() + 7
    while any(item.get("accessibleRole") == "Banner" for item in tree()):
        assert time.monotonic() < deadline, "focus check did not restore the connected shell"
        time.sleep(.05)
    assert focus_label() == "Find a conversation"


def smoke_chat(label):
    show_chats()
    call("click_element", elementHandle=element(label, "Button")["handle"])
    deadline = time.monotonic() + 2
    def selected_chat_visible():
        items = tree()
        if properties["size"]["width"] / properties.get("scaleFactor", 1) < 740:
            return any(item.get("accessibleRole") == "Button" and item.get("accessibleLabel") == label for item in items) and any(item.get("accessibleLabel") == "Message draft" for item in items)
        return any(item.get("accessibleItemSelected") and item.get("accessibleLabel", "").startswith(label + ", ") for item in items)
    while not selected_chat_visible():
        assert time.monotonic() < deadline, f"chat selection did not settle: {label}"
        time.sleep(.02)

show_chats()
search = element("Find a conversation", "TextInput")["handle"]
draft = "日本語の下書き\nDraft with emoji 🐈"
smoke_chat("葵")
composer = element("Message draft", "TextInput")["handle"]
call("set_element_value", elementHandle=composer, value=draft)
smoke_chat("Weekend plans")
call("set_element_value", elementHandle=composer, value="Another draft")
smoke_chat("葵")
value = call("get_element_properties", elementHandle=composer)
assert value.get("accessibleValue") == draft, "draft did not survive chat switching"
show_chats()
call("set_element_value", elementHandle=search, value="Weekend")
chat_rows = [item for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith(("葵, ", "Weekend plans, ", "Notes to self, "))]
assert len(chat_rows) == 1 and chat_rows[0]["accessibleLabel"].startswith("Weekend plans, "), "search did not filter chats"
call("set_element_value", elementHandle=search, value="")
smoke_chat("葵")
assert element("Send", "Button").get("accessibleEnabled", False), "demo transport did not enable Send"
call("click_element", elementHandle=composer)
call("dispatch_key_event", windowHandle=window, text="\n")
value = call("get_element_properties", elementHandle=composer)
assert value.get("accessibleValue", "").endswith("\n"), "Enter did not insert a newline"
before_shortcut = value.get("accessibleValue")
call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Press")
call("dispatch_key_event", windowHandle=window, text="\n")
call("dispatch_key_event", windowHandle=window, text="\x11", eventType="Release")
deadline = time.monotonic() + 2
while call("get_element_properties", elementHandle=composer).get("accessibleValue", ""):
    assert time.monotonic() < deadline, "durable send acknowledgement did not clear the draft"
    time.sleep(.02)
assert call("get_element_properties", elementHandle=composer).get("accessibleValue", "") == "", "Ctrl+Enter did not clear the submitted draft"
assert any(item.get("accessibleValue") == before_shortcut for item in tree()), "submitted draft did not appear in history"
element("Sent", "Image")
call("set_element_value", elementHandle=composer, value=before_shortcut)
if args.compose:
    def compose_click(label, role="Button"):
        call("click_element", elementHandle=element(label, role)["handle"])

    def compose_key(text):
        call("dispatch_key_event", windowHandle=window, text=text)

    def surface_shot(name):
        if args.surface_shots:
            args.surface_shots.mkdir(parents=True, exist_ok=True)
            result = rpc("tools/call", {"name":"take_screenshot", "arguments":{"windowHandle":window}})
            image = next(item for item in result["content"] if item["type"] == "image")
            (args.surface_shots / f"{name}.png").write_bytes(base64.b64decode(image["data"]))

    # Select the just-sent message; its reaction target is visible at the tail.
    def open_reactions():
        reaction = next(item for item in reversed(tree()) if item.get("accessibleLabel") == "React" and item.get("accessibleRole") == "Button")
        call("click_element", elementHandle=reaction["handle"])
    open_reactions()
    element("LOVE", "Button")
    compose_click("LOVE")
    open_reactions()
    wait_for(lambda: element("LOVE", "Button").get("accessibleItemSelected"), "own reaction was not highlighted")
    surface_shot("reactions")
    compose_click("LOVE")
    open_reactions()
    wait_for(lambda: not element("LOVE", "Button").get("accessibleItemSelected"), "repeat pick did not cancel")
    compose_key("\x1b")
    compose_click("Stickers")
    element("Send sticker 1", "Button")
    wait_for(lambda: len([item for item in tree() if item.get("accessibleRole") == "Tab"]) >= 2, "owned packages missing")
    wait_for(lambda: element("Sticker preview 1", "Image").get("accessibleDescription") == "Loaded", "picker images did not load")
    surface_shot("stickers")
    compose_click("Send sticker 1")
    wait_for(lambda: any("Sticker" in item.get("accessibleLabel", "") and item.get("accessibleRole") in ("Text", "Image") for item in tree()), "sticker did not reach history")
    assert not any(item.get("accessibleLabel") == "Send sticker 1" for item in tree()), "picker stayed open after send"
    compose_click("Stickers")
    wait_for(lambda: element("Sticker preview 1", "Image").get("accessibleDescription") == "Loaded", "cached picker did not reopen")
    compose_key("\x1b")
    assert not any(item.get("accessibleLabel") == "Send sticker 1" for item in tree()), "Escape did not close sticker picker"
    show_chats()
    compose_click("New chat")
    element("Find friends for new chat", "TextInput")
    surface_shot("new-chat")
    compose_click("新しい友だち 🌸", "Checkbox")
    compose_click("Chat")
    wait_for(lambda: any(item.get("accessibleLabel") == "新しい友だち 🌸" and item.get("accessibleRole") == "Button" for item in tree()), "friend conversation did not open")
    draft_control = element("Message draft", "TextInput")
    call("click_element", elementHandle=draft_control["handle"])
    for char in "Hello from a new chat": compose_key(char)
    wait_for(lambda: element("Message draft", "TextInput").get("accessibleValue") == "Hello from a new chat", "new friend composer did not accept typed text")
    compose_click("Send")
    wait_for(lambda: any(item.get("accessibleValue") == "Hello from a new chat" for item in tree()), "first send to a friend without a chat failed")
    show_chats()
    wait_for(lambda: any(item.get("accessibleLabel", "").startswith("新しい友だち 🌸, ") for item in tree()), "first-send chat row missing")
    compose_click("New chat")
    compose_click("葵", "Checkbox")
    compose_click("新しい友だち 🌸", "Checkbox")
    call("set_element_value", elementHandle=element("Group name", "TextInput")["handle"], value="Compose test group")
    surface_shot("new-group")
    compose_click("Create group")
    wait_for(lambda: any(item.get("accessibleLabel") == "Compose test group" for item in tree()), "created group did not open")
    # The portal is unavailable in headless runs; this guarded field injects local paths.
    import struct, zlib
    with tempfile.TemporaryDirectory(prefix="line-attach-ui-") as directory:
        directory = Path(directory)
        def png_chunk(kind, data):
            return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
        photo = directory / "Holiday.png"
        scanlines = b"".join(b"\0" + bytes([58, 145, 191]) * 160 for _ in range(120))
        photo.write_bytes(b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB",160,120,8,2,0,0,0)) + png_chunk(b"IDAT", zlib.compress(scanlines)) + png_chunk(b"IEND", b""))
        file = directory / "Notes.txt"; file.write_text("Synthetic file")
        removed = directory / "Remove.txt"; removed.write_text("Remove this item")
        call("dispatch_key_event", windowHandle=window, text="\uf70f")
        # Reopening retains the scroll position from earlier scenario checks.
        close = element("Close demo scenarios", "Button")
        call("dispatch_pointer_scroll", windowHandle=window,
             position={"x": close["absolutePosition"]["x"] + close["size"]["width"] / 2,
                       "y": close["absolutePosition"]["y"] + close["size"]["height"] + 60},
             deltaY=10000)
        injection = element("Attachment paths", "TextInput")
        call("set_element_value", elementHandle=injection["handle"], value=json.dumps([str(photo), str(file), str(removed)]))
        compose_click("Preview attachments")
        element("Send 3", "Button")
        surface_shot("attachments")
        compose_click("Remove Remove.txt")
        element("Send 2", "Button")
        assert not any(item.get("accessibleLabel", "").startswith("Uploading") for item in tree()), "selection uploaded before Send"
        compose_click("Send 2")
        wait_for(lambda: any(item.get("accessibleLabel", "").startswith("Uploading ") and item.get("accessibleLabel") not in ("Uploading 0%", "Uploading 100%") for item in tree()), "upload progress did not appear")
        surface_shot("uploading")
        wait_for(lambda: any(item.get("accessibleValue") == "Notes.txt" for item in tree()) and not any(item.get("accessibleLabel", "").startswith("Uploading") or item.get("accessibleLabel") == "Sending" for item in tree()), "attachment rows did not finish")
    print("PASS: react/cancel, stickers, new chats, attachment preview/remove/send/progress")

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
    # The fixture page arrives on the worker. At wide size the original short
    # chat fits entirely, so scrolling must wait until the long page is committed.
    deadline = time.monotonic() + 5
    while True:
        call("dispatch_pointer_scroll", windowHandle=window, position={"x": 600, "y": 280}, deltaY=800, deltaX=0)
        if any(item.get("accessibleLabel") == "Latest" for item in tree()): break
        assert time.monotonic() < deadline, "long history did not become scrollable"
        time.sleep(.05)
    body = next(item for item in bodies() if "\n" in item.get("accessibleValue", "") and item["absolutePosition"]["y"] > 110)
    expected_text = body["accessibleValue"]
    expected_y = body["absolutePosition"]["y"]
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
    wait_for(lambda: call("get_element_properties", elementHandle=composer).get("accessibleValue") == expected_text, "clipboard lost selected text/emoji or included metadata")
    call("click_element", elementHandle=element("Latest · 3 new", "Button")["handle"])
    assert not any("Latest" in item.get("accessibleLabel", "") for item in tree() if item.get("accessibleRole") == "Button"), "Latest did not resume following"
    demo_scenario("Receive 3")
    time.sleep(0.5)
    assert any("到着 3" in item.get("accessibleValue", "") for item in bodies()), "at-latest burst did not remain visible"
    assert not any("Latest" in item.get("accessibleLabel", "") for item in tree() if item.get("accessibleRole") == "Button"), "at-latest burst disabled following"
    call("set_element_value", elementHandle=composer, value="Keep this newer draft")
    demo_scenario("Uncertain send")
    time.sleep(1.2)
    assert not any(item.get("accessibleRole") == "Button" and item.get("accessibleLabel") in ("Retry connection", "Retry conversations", "Retry older") for item in tree()), "removed network Retry control returned"
    assert any("Maybe not sent" in item.get("accessibleLabel", "") for item in tree()), "uncertain result was not attached to its outgoing item"
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == "Keep this newer draft", "late result replaced newer text"
    failed = next(item for item in bodies() if "Unconfirmed" in item.get("accessibleLabel", ""))
    call("click_element", elementHandle=failed["handle"], button="Right")
    call("click_element", elementHandle=element("Copy to composer", "Text")["handle"])
    call("click_element", elementHandle=element("Append", "Button")["handle"])
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue", "").startswith("Keep this newer draft\n"), "recovery overwrote the draft"
    print("PASS: simulated pointer scrolling, burst anchor/count, keyboard selection/clipboard, uncertain-send recovery")

if args.layout_photos:
    assert not any(item.get("accessibleLabel") in ("Smaller text", "Larger text", "Text size") for item in tree()), "text-size controls leaked onto the main screen"
    avatars = [item for item in tree() if item.get("accessibleLabel") == "Contact photo"]
    assert avatars and all(item["size"] == {"height": 44.0, "width": 44.0} for item in avatars), "avatar escaped its frame"
    call("click_element", elementHandle=element("Settings", "Button")["handle"])
    element("Message text size", "Combobox")
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    element("Settings", "Button")
    before = call("get_element_properties", elementHandle=composer).get("accessibleValue")
    demo_scenario("Photo preview")
    preview = element("Photo preview", "Image")
    timestamp = next(item for item in tree() if item.get("accessibleRole") == "Text" and item.get("accessibleLabel", "") in fixture_times("12:05"))
    assert preview["size"]["height"] == 180, "photo exceeded reserved preview height"
    assert timestamp["absolutePosition"]["x"] >= preview["absolutePosition"]["x"] + preview["size"]["width"], "timestamp overlapped the photo instead of sitting beside it"
    assert timestamp["size"]["height"] <= 32, "timestamp exceeded a single line at the supported time type scale"
    call("click_element", elementHandle=element("Open photo", "Button")["handle"])
    element("Close photo", "Button")
    assert any("photo-landscape · 1200 × 800" in item.get("accessibleLabel", "") for item in tree()), "viewer did not request original resolution"
    call("click_element", elementHandle=element("Original size", "Button")["handle"])
    original = element("Photo in viewer", "Image")
    assert original["size"]["width"] >= 1199, "original-size view only enlarged the thumbnail"
    call("click_element", elementHandle=element("Fit", "Button")["handle"])
    fitted=element("Photo in viewer", "Image")["size"]
    assert fitted["width"] <= properties["size"]["width"] + 1 and fitted["height"] <= properties["size"]["height"] + 1, "Fit did not restore a bounded view"
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == before, "viewer changed the draft"
    element("Open photo", "Button")
    print("PASS: bounded round-avatar frames, settings placement, media/timestamp separation, original viewer, zoom/fit, Escape, and draft retention")
    demo_scenario("Sticker preview")
    element("Sticker · [sticker]", "Text")
    sticker_time = next(item['accessibleLabel'] for item in tree() if item.get('accessibleRole') == 'Text' and item.get('accessibleLabel') in fixture_times('12:09'))
    before_sticker_time = element(sticker_time, "Text")["absolutePosition"]
    time.sleep(1.1)
    element("Sticker", "Image")
    assert not any(item.get("accessibleLabel") == "Sticker · [sticker]" or item.get("accessibleValue") == "Sticker · [sticker]" for item in tree()), "loaded sticker retained its fallback label"
    assert element(sticker_time, "Text")["absolutePosition"] == before_sticker_time, "sticker completion moved its timestamp"
    print("PASS: sticker fallback disappears after loading without moving the timestamp")
    demo_scenario("Group preview")
    element("Members", "Button")  # The group's details arrive after the fixture history.
    # Inspect both ends because the final timestamp can be clipped. Scope to
    # history: a browser relative time may also legitimately say "Yesterday".
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 600, "y": 280}, deltaY=2000, deltaX=0)
    grouped = tree()
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 600, "y": 280}, deltaY=-2000, deltaX=0)
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
    def day_label(date):
        year, month, day = date.year, date.month, date.day
        age = (datetime.date.today() - date).days
        return "Today" if age == 0 else "Yesterday" if age == 1 else date.strftime("%A") + f" {month}/{day}" if 2 <= age <= 6 else f"{year}/{month}/{day}"
    assert labels.count(day_label(datetime.date.today() - datetime.timedelta(days=1))) == 1 and labels.count(day_label(datetime.date.today())) == 1, "calendar dates were repeated or omitted"
    times = set()
    for visible_group in (history_rows(grouped), history_rows(scrolled_group)):
        message_bodies = [item for item in visible_group if item.get("accessibleRole") == "TextInput" and any(base.get("id", "").endswith("::body") for base in item.get("typeNamesAndIds", []))]
        for meta in visible_group:
            label = meta.get("accessibleLabel", "")
            if not (len(label) == 16 and label[4] == "/" and label[10] == " "):
                continue
            times.add(label)
            assert any(meta["absolutePosition"]["x"] >= body["absolutePosition"]["x"] + body["size"]["width"] and abs(meta["absolutePosition"]["y"] - body["absolutePosition"]["y"]) <= body["size"]["height"] + 12 for body in message_bodies), "group time added a footer row"
    yesterday_times = {(datetime.date.today() - datetime.timedelta(days=1)).strftime("%Y/%m/%d") + " " + clock for clock in ("09:18", "09:20", "09:21", "09:27")}
    assert len(times) == 5 and yesterday_times <= times and times - yesterday_times <= fixture_times("09:18"), "five-minute/sender/day grouping showed the wrong timestamps"
    print("PASS: sender avatars/names, five-minute grouping, day boundaries, and compact time/status beside bubbles")

def check_detail_stack():
    def click(label, role="Button"):
        if label.startswith("Open member: "):
            call("set_element_value", elementHandle=element("Search members", "TextInput")["handle"],value=label.removeprefix("Open member: "))
        call("click_element", elementHandle=element(label, role)["handle"])
    def key(text):
        call("dispatch_key_event", windowHandle=window, text=text)
    def focused(label, role="Button"):
        assert "Focused" in element(label, role).get("accessibleDescription", ""), f"focus not restored to {label}: {[item.get('accessibleLabel') for item in tree() if 'Focused' in item.get('accessibleDescription','')]}"
    click("Friends")
    search=element("Search friends and groups", "TextInput")
    call("set_element_value",elementHandle=search["handle"],value="")
    click("Open profile: Weekend plans")
    element("Group info", "Text")
    assert any(item.get("accessibleLabel")=="Weekend plans" for item in tree()), "group info is not bound to its chat"
    click("Members (25)")
    element("Search members", "TextInput")
    click("Open member: 葵")
    element("Profile", "Text")
    click("Rename")
    call("set_element_value", elementHandle=element("Name", "TextInput")["handle"], value="Aoi test 🌿")
    click("Save")
    element("Aoi test 🌿", "Text")
    element("葵", "Text")
    click("Rename")
    call("set_element_value", elementHandle=element("Name", "TextInput")["handle"], value="葵")
    click("Save")
    element("葵", "Text")
    click("Open profile photo")
    element("Photo in viewer", "Image")
    key("\x1b")
    focused("Open profile photo")
    click("Back")
    focused("Group members", "List")
    click("Open member: 蓮")
    click("Add friend")
    element("Rename", "Button")
    element("Chat", "Button")
    click("Back")
    click("Open member: Unknown member")
    assert not any(item.get("accessibleLabel") in ("Chat", "Rename", "Add friend") for item in tree()), "unknown profile exposed a write"
    click("Back")
    call("set_element_value", elementHandle=element("Search members", "TextInput")["handle"], value="葵")
    click("Back")
    focused("Members (25)")
    click("Members (25)")
    click("Open member: 葵")
    key("\x1b")
    focused("Group members", "List")
    key("\x1b")
    focused("Members (25)")
    click("Members (25)")
    click("Open member: 葵")
    click("Close details")
    assert not any(item.get("accessibleLabel")=="Close details" for item in tree()), "close did not clear the stack"
    focused("Friends and groups", "List")
    click("Open profile: Weekend plans")
    click("Notifications", "Checkbox")
    deadline = time.monotonic() + 3
    while element("Notifications", "Checkbox").get("accessibleValue") != "Off":
        assert time.monotonic() < deadline, "mute did not update"
        time.sleep(.02)
    click("Leave group")
    element("Leave group?", "Text")
    key("\x1b")
    click("Leave group")
    click("Leave")
    click("Close details")
    click("Chats")
    smoke_chat("葵")
    print("PASS: group info → members → profile, Back/Escape pop, × closes stack, opener focus restoration")

if args.friends:
    check_detail_stack()
    call("set_element_value", elementHandle=composer, value=draft)
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    friend_search = element("Search friends and groups", "TextInput")["handle"]
    element("Open profile: 葵", "Button")  # Friends load on the engine worker.
    call("set_element_value", elementHandle=friend_search, value="葵")
    friend_rows = [item for item in tree() if item.get("accessibleLabel", "").startswith("Open profile:")]
    assert len(friend_rows) == 1, "friend name search failed"
    call("click_element", elementHandle=friend_search)
    call("dispatch_key_event", windowHandle=window, text="\uf701")  # Category header
    call("dispatch_key_event", windowHandle=window, text="\uf701")  # First friend
    call("dispatch_key_event", windowHandle=window, text="\n")
    element("Profile", "Text")
    assert any(item.get("accessibleLabel") == "Message draft" for item in tree()) == args.wide, "profile composer visibility disagrees with panel/sheet layout"
    call("click_element", elementHandle=element("Chats", "Button")["handle"])
    assert not any(item.get("accessibleLabel") == "Close details" for item in tree()), "destination switch left profile open"
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    call("click_element", elementHandle=element("Open profile: 葵", "Button")["handle"])
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    element("Search friends and groups", "TextInput")
    call("click_element", elementHandle=element("Chats", "Button")["handle"])
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == draft, "friend browsing lost draft"
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    assert call("get_element_properties", elementHandle=friend_search).get("accessibleValue") == "葵", "section switch lost friend query"
    call("set_element_value", elementHandle=friend_search, value="")
    call("click_element", elementHandle=element("Groups", "Button")["handle"])
    assert not any(item.get("accessibleLabel", "").startswith("Open profile:") and "Weekend plans" in item.get("accessibleLabel", "") for item in tree()), "collapsed groups remained visible"
    call("click_element", elementHandle=element("Groups", "Button")["handle"])
    call("click_element", elementHandle=element("Open profile: 新しい友だち 🌸", "Button")["handle"])
    call("click_element", elementHandle=element("Chat", "Button")["handle"])
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue", "") == "", "new friend inherited another draft"
    element("No messages yet", "Text")
    assert element("Chats", "Button").get("accessibleItemSelected") is True, "Open chat left the rail on Friends"
    element("新しい友だち 🌸", "Button")  # local conversation header
    assert not any(item.get("accessibleLabel", "").startswith("新しい友だち 🌸, ") for item in tree()), "opening a friend created a chat row before its first send"
    for label in ("葵", "Weekend plans", "Notes to self"):
        element(label, "Button")
    demo_scenario("Long chat list")
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    call("click_element", elementHandle=element("Open profile: 新しい友だち 🌸", "Button")["handle"])
    call("click_element", elementHandle=element("Chat", "Button")["handle"])
    row = wait_for(lambda: next((item for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith("新しい友だち 🌸, ")), None), "far chat row was not revealed")
    viewport = element("Conversations", "List")
    assert row["absolutePosition"]["y"] >= viewport["absolutePosition"]["y"], "far row was not revealed"
    assert row["absolutePosition"]["y"] + row["size"]["height"] <= viewport["absolutePosition"]["y"] + viewport["size"]["height"] + 1, "far row remained clipped"
    assert abs(row["absolutePosition"]["y"] + row["size"]["height"] - viewport["absolutePosition"]["y"] - viewport["size"]["height"]) <= 1, "Open chat did not use the minimum scroll to reveal the far row"
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    call("set_element_value", elementHandle=friend_search, value="葵")
    call("click_element", elementHandle=element("Open profile: 葵", "Button")["handle"])
    call("click_element", elementHandle=element("Chat", "Button")["handle"])
    call("click_element", elementHandle=element("葵", "Button")["handle"])
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == draft, "opening a new friend lost prior draft"
    print("PASS: friend search keyboard activation, profile Escape/destination closure, retained query/categories, Chats selection/empty state, new-chat isolation and prior draft")

if args.auth_recovery:
    before = call("get_element_properties", elementHandle=composer).get("accessibleValue")
    tracked = [("Find a conversation", "TextInput"), ("Conversations", "List"), ("Message history", "List"), ("Message draft", "TextInput")]
    positions = {label: element(label, role)["absolutePosition"]["y"] for label, role in tracked}

    def unchanged_shell():
        current = {label: element(label, role)["absolutePosition"]["y"] for label, role in tracked}
        assert current == positions, f"connection feedback moved shell content: {positions} -> {current}"

    def safe_strip():
        strip = element("Connection needs attention", "Banner")
        history = element("Message history", "List")
        assert strip["absolutePosition"]["y"] == history["absolutePosition"]["y"], "feedback is not below the conversation header"
        assert strip["size"]["height"] >= 32, "connection feedback is too small"
        for label, role in [("Find a conversation", "TextInput"), ("Message draft", "TextInput"), ("Send", "Button")]:
            control = element(label, role)
            a, b = strip["absolutePosition"], control["absolutePosition"]
            size_a, size_b = strip["size"], control["size"]
            assert (a["x"] + size_a["width"] <= b["x"] or b["x"] + size_b["width"] <= a["x"]
                    or a["y"] + size_a["height"] <= b["y"] or b["y"] + size_b["height"] <= a["y"]), f"feedback covered {label}"

    demo_scenario("Offline")
    safe_strip()
    unchanged_shell()
    assert not any(item.get("accessibleLabel") == "Sign in again" for item in tree()), "network failure offered sign-in"
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == before
    demo_scenario("Expired sign-in")
    safe_strip()
    unchanged_shell()
    call("click_element", elementHandle=composer)
    call("dispatch_key_event", windowHandle=window, text="\uf709")  # F6 enters sign-in recovery.
    assert element("Sign in again", "Button").get("accessibleDescription") == "Focused"
    call("dispatch_key_event", windowHandle=window, text="\n")
    assert any("Signing in here signs LINE out in Chrome." in item.get("accessibleLabel", "") for item in tree())
    assert element("Start", "Button").get("accessibleDescription") == "Focused"
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    assert element("Sign in again", "Button").get("accessibleDescription") == "Focused"
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == before, "sign-in dismissal changed the draft"
    call("dispatch_key_event", windowHandle=window, text="\t")
    assert element("Message history", "List").get("accessibleDescription") == "Focused history"
    demo_scenario("Reconnect blip")
    deadline = time.monotonic() + 5
    while any(item.get("accessibleLabel") == "Connection needs attention" for item in tree()):
        assert time.monotonic() < deadline, "automatic reconnect did not clear recovery"
        time.sleep(.05)
    unchanged_shell()
    print("PASS: offline/expired feedback placement, F6/Tab order, sign-in dismissal and draft preservation")

if args.layers:
    check_detail_stack()
    demo_scenario("Protocol fixtures")
    # Public OA metadata assets use the same offline media paths as other photos.
    call("click_element", elementHandle=element("Open photo", "Button")["handle"])
    element("Photo in viewer", "Image")
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    demo_scenario("Live reaction")
    element("👍", "Text")
    element("1", "Text")

    def unsend_menu(body):
        message = next(item for item in tree() if body in item.get("accessibleLabel", "") and item.get("accessibleRole") in ("Text", "TextInput"))
        call("click_element", elementHandle=message["handle"], button="Right")
        call("click_element", elementHandle=element("Unsend", "Text")["handle"])
        element("Unsend message?", "Text")
        element("It will be removed for everyone.", "Text")
        assert element("Cancel", "Button").get("accessibleDescription", "") in ("", "Focused")
        draft_before = call("get_element_properties", elementHandle=composer).get("accessibleValue")
        call("dispatch_key_event", windowHandle=window, text="x")
        assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == draft_before, "Unsend let typing reach the draft"

    unsend_menu("Unsend success fixture")
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    unsend_menu("Unsend success fixture")
    call("click_element", elementHandle=element("Cancel", "Button")["handle"])
    unsend_menu("Unsend failure fixture")
    call("click_element", elementHandle=element("Unsend", "Button")["handle"])
    element("Couldn't unsend.", "Text")
    assert any("Unsend failure fixture" in item.get("accessibleLabel", "") for item in tree())
    unsend_menu("Unsend success fixture")
    call("click_element", elementHandle=element("Unsend", "Button")["handle"])
    deadline = time.monotonic() + 2
    while not any("Message unsent" in item.get("accessibleLabel", "") for item in tree()):
        assert time.monotonic() < deadline, "Unsend success did not replace the row"
        time.sleep(.05)
    print("PASS: protocol OA image/original, quiet live reaction, Unsend menu, Escape/Cancel, failure and success")
    def bounds_by_id(qualified_id):
        handles = call("find_elements_by_id", windowHandle=window, elementsId=qualified_id)["elementHandles"]
        assert len(handles) == 1, f"non-unique geometry: {qualified_id}"
        return call("get_element_properties", elementHandle=handles[0])

    rail = bounds_by_id("AppWindow::rail")
    browser = bounds_by_id("AppWindow::browser")
    browser_edge = rail["size"]["width"] + browser["size"]["width"]
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
    friend_search = element("Search friends and groups", "TextInput")["handle"]
    call("set_element_value", elementHandle=friend_search, value="葵")
    call("click_element", elementHandle=element("Open profile: 葵", "Button")["handle"])
    profile_close_y = element("Close details", "Button")["absolutePosition"].get("y", 0)
    profile = bounds_by_id("Layers::profile")
    if args.wide:
        assert profile["absolutePosition"].get("x", 0) > browser_edge, "panel covered the conversation"
    else:
        assert abs(profile["absolutePosition"].get("x", 0) - browser_edge) <= 1, "sheet did not start after rail and browser"
    assert any(item.get("accessibleLabel") == "Message draft" for item in tree()) == args.wide, "profile conversation visibility disagrees with layout"
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
    assert element("Photo in viewer", "Image")["size"]["width"] <= properties["size"]["width"] + 1 and element("Photo in viewer", "Image")["size"]["height"] <= properties["size"]["height"] + 1, "0 did not fit the photo"
    call("click_element", elementHandle=element("Save photo", "Button")["handle"])
    for _ in range(100):
        saved = [item.get("accessibleLabel", "") for item in tree() if item.get("accessibleLabel", "").startswith("Saved to ")]
        if saved: break
    assert saved, "Save did not report a completed export"
    exported = Path(saved[0].removeprefix("Saved to "))
    assert exported.is_relative_to(Path(tempfile.gettempdir()) / "line-desktop-demo-exports"), "preview export escaped its isolated folder"
    assert exported.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
    exported.unlink()
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    assert any(item.get("accessibleLabel") == "Open photo" and item.get("accessibleDescription") == "Focused" for item in tree()), "viewer did not restore the photo opener"
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == before, "layers changed the draft"
    demo_scenario("Expired sign-in")
    call("click_element", elementHandle=element("Friends", "Button")["handle"])
    call("click_element", elementHandle=element("Open profile: 葵", "Button")["handle"])
    close = element("Close details", "Button")
    assert bounds_by_id("Layers::profile")["absolutePosition"].get("x", 0) == profile["absolutePosition"].get("x", 0)
    assert close["absolutePosition"].get("y", 0) == profile_close_y, "connection feedback moved the profile"
    assert any(item.get("accessibleLabel") == "Connection needs attention" for item in tree()) == args.wide, "profile recovery visibility disagrees with panel/sheet layout"
    browser_search = element("Search friends and groups", "TextInput")
    assert browser_search["absolutePosition"]["x"] + browser_search["size"]["width"] <= close["absolutePosition"]["x"], "profile hid the browser"
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    call("click_element", elementHandle=element("Chats", "Button")["handle"])
    call("click_element", elementHandle=element("Weekend plans", "Button")["handle"])
    strip = element("Connection needs attention", "Banner")
    strip_bottom = strip["absolutePosition"].get("y", 0) + strip["size"]["height"]
    call("click_element", elementHandle=element("Members", "Button")["handle"])
    element("Members (25)", "Text")
    for name in ("葵", "蓮"):
        element("Open member: " + name, "Button")
    members = bounds_by_id("Layers::profile")
    if args.wide:
        assert members["absolutePosition"].get("x", 0) > browser_edge, "members panel covered the conversation"
        assert members["absolutePosition"].get("y", 0) == 0, "members panel moved for the recovery strip"
    else:
        assert abs(members["absolutePosition"].get("x", 0) - browser_edge) <= 1, "members sheet covered the browser"
        assert members["absolutePosition"].get("y", 0) == 0, "detail sheet changed its position for recovery"
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    print("PASS: Settings modal focus, profile reading/focus/browser/connection-strip placement, viewer keys/wheel/pan/loaded-photo navigation, isolated PNG export, opener restoration, draft retention")

if args.reading:
    def reading_key(text):
        call("dispatch_key_event", windowHandle=window, text=text)

    def reading_bodies():
        return [item for item in tree() if item.get("accessibleRole") in ("TextInput", "Text") and item.get("accessibleLabel", "").startswith(("Incoming from ", "Outgoing, "))]

    demo_scenario("Reading fixtures")
    deadline = time.monotonic() + 5
    while not any(item.get("accessibleLabel") == "Beginning of conversation" for item in tree()):
        assert time.monotonic() < deadline, "automatic reading-window refresh did not finish"
        time.sleep(.05)
    labels = [item.get("accessibleLabel", "") for item in tree()]
    # MCP exposes clipped rows only while visible; inspect both fixture halves.
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 650, "y": 280}, deltaY=-1800, deltaX=0)
    labels.extend(item.get("accessibleLabel", "") for item in tree())
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 650, "y": 280}, deltaY=1800, deltaX=0)
    for expected in ("Photo", "Sticker", "Video", "Voice message", "File", "Location", "Contact", "System message", "Message unsent", "Couldn't decrypt this message", "Unsupported message"):
        assert any(expected in label for label in labels), f"missing placeholder: {expected}"
    first = next(item for item in reading_bodies() if "https://example.org/reading" in item.get("accessibleValue", ""))
    first_y = first["absolutePosition"]["y"]
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
    last = next(item for item in reading_bodies() if "Unsupported message" in item.get("accessibleValue", ""))
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
    print(f"PASS: reading search→composer in {tabs} Tabs, single history focus/scroll, demo link activation and 12 content kinds")

if args.rich:
    demo_scenario("Rich content")
    def rich_tree():
        result = call("get_element_tree", elementHandle=root, maxElements=2500)
        assert not result.get("truncated"), "rich fixture exceeded bounded tree"
        return result["elements"]
    def rich_element(label, role):
        return element(label, role, rich_tree)
    emoji_image = rich_element("[笑]", "Image")
    rich_body = next(item for item in rich_tree() if item.get("accessibleRole") == "Text" and "日😀[笑]本🌸" in item.get("accessibleLabel", ""))
    assert rich_body["size"]["height"] > emoji_image["size"]["height"] * 2, "rich text did not wrap into measured lines"
    assert emoji_image["absolutePosition"]["x"] > rich_body["absolutePosition"]["x"], "inline image overlapped the first text run"
    metadata_rows = rich_tree()
    labels = [item.get("accessibleLabel", "") for item in metadata_rows]
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 800, "y": 400}, deltaY=-800, deltaX=0)
    metadata_rows += rich_tree()
    labels += [item.get("accessibleLabel", "") for item in metadata_rows]
    assert any(item.get("accessibleValue") == "葵" and any(base.get("id", "").endswith("::card-title") for base in item.get("typeNamesAndIds", [])) for item in metadata_rows), "contact card lost its name"
    for expected in ("2.0 KB", "2:05", "0:12", "葵", "葵 joined the group", "Message unsent"):
        assert any(expected in label for label in labels), f"missing rich metadata: {expected}"
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 800, "y": 400}, deltaY=1600, deltaX=0)
    reply = next(item for item in rich_tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel") == "Jump to replied message")
    def emoji_body():
        return next(item for item in rich_tree() if item.get("accessibleRole") == "Text" and "日😀[笑]本🌸" in item.get("accessibleLabel", ""))
    if any(item.get("accessibleRole")=="Button" and item.get("accessibleLabel","").startswith("Latest") for item in rich_tree()):
        for _ in range(2):
            latest = next(item for item in rich_tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith("Latest"))
            call("click_element", elementHandle=latest["handle"])
            assert not any(item.get("accessibleLabel") == "Return to message" for item in rich_tree()), "Latest retained the return anchor"
            latest_y = emoji_body()["absolutePosition"]["y"]
            call("click_element", elementHandle=rich_element("Jump to replied message", "Button")["handle"])
            wait_for(lambda: abs(emoji_body()["absolutePosition"]["y"] - latest_y) > 1, "reply jump after Latest did not scroll")
            call("click_element", elementHandle=rich_element("Return to message", "Button")["handle"])
            wait_for(lambda: abs(emoji_body()["absolutePosition"]["y"] - latest_y) <= 1, "Return after Latest lost its origin")
        print("PASS: Latest → reply jump → Return → Latest → reply jump → Return")
        call("dispatch_pointer_scroll", windowHandle=window, position={"x": 800, "y": 400}, deltaY=1600, deltaX=0)
        reply = rich_element("Jump to replied message", "Button")
        origin_y = emoji_body()["absolutePosition"]["y"]
        call("click_element", elementHandle=reply["handle"])
        demo_scenario("Offline")
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
        for label, role in [("Return to message", "Button"), (latest["accessibleLabel"], "Button"), ("Message history", "List")]:
            call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Press")
            call("dispatch_key_event", windowHandle=window, text="\t")
            call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Release")
            assert rich_element(label, role).get("accessibleDescription", "").startswith("Focused"), f"reverse Tab skipped {label}"
        assert rich_element("Message history", "List").get("accessibleDescription") == "Focused history", "reverse Tab skipped a history pill"
        for label, role in [(latest["accessibleLabel"], "Button"), ("Return to message", "Button"), ("Message draft", "TextInput")]:
            call("dispatch_key_event", windowHandle=window, text="\t")
            assert rich_element(label, role).get("accessibleDescription", "").startswith("Focused"), f"forward Tab skipped {label}"
        assert call("get_element_properties", elementHandle=composer).get("accessibleDescription") == "Focused message draft", "history/Latest/Return/composer Tab order changed"
        call("click_element", elementHandle=rich_element("Return to message", "Button")["handle"])
        wait_for(lambda: abs(emoji_body()["absolutePosition"]["y"] - origin_y) <= 1, "reply return did not restore its origin")
        restored_y = emoji_body()["absolutePosition"]["y"]
        assert abs(restored_y - origin_y) <= 1, f"Offline appearing moved reply return: {origin_y} -> {restored_y}"
        print(f"PASS: reply return with Offline appearing: y={origin_y} -> {restored_y}")
        # Capture with the strip present, then let the local reconnect fixture clear it.
        origin_y = emoji_body()["absolutePosition"]["y"]
        reply = rich_element("Jump to replied message", "Button")
        quote_y = reply["absolutePosition"]["y"]
        call("click_element", elementHandle=reply["handle"])
        demo_scenario("Reconnect blip")
        deadline = time.monotonic() + 7
        while any(item.get("accessibleRole") == "Banner" and item.get("accessibleLabel") in ("Connection needs attention", "Connection restored") for item in rich_tree()):
            assert time.monotonic() < deadline, "synthetic reconnect did not clear the strip"
            time.sleep(0.1)
        call("click_element", elementHandle=rich_element("Return to message", "Button")["handle"])
        wait_for(lambda: abs(emoji_body()["absolutePosition"]["y"] - origin_y) <= 1, "reply return did not restore its origin")
        restored_y = emoji_body()["absolutePosition"]["y"]
        assert abs(restored_y - origin_y) <= 1, f"Offline disappearing moved reply return: {origin_y} -> {restored_y}"
        reply = rich_element("Jump to replied message", "Button")
        assert abs(reply["absolutePosition"]["y"] - quote_y) <= 1, "Return moved the reply quote"
        history = rich_element("Message history", "List")
        viewports = call("find_elements_by_id", windowHandle=window, elementsId="Pane::viewport")["elementHandles"]
        viewport = next(item for handle in viewports
                        if -1 <= (item := call("get_element_properties", elementHandle=handle))["absolutePosition"].get("x", 0) - history["absolutePosition"].get("x", 0) <= 16
                        and abs(item["size"]["width"] - (history["size"]["width"] - 12)) <= 1)
        assert abs(viewport["absolutePosition"]["y"] - history["absolutePosition"]["y"]) <= 1, "Return left a blank residual band above the anchor"
        print(f"PASS: reply return with Offline disappearing: y={origin_y} -> {restored_y}")
        # Retain the unchanged-inset regression as well.
        origin_y = restored_y
        call("click_element", elementHandle=rich_element("Jump to replied message", "Button")["handle"])
        rich_element("Return to message", "Button")  # The restored quote must accept another jump.
        call("click_element", elementHandle=rich_element("Return to message", "Button")["handle"])
        wait_for(lambda: abs(emoji_body()["absolutePosition"]["y"] - origin_y) <= 1, "reply return lost the reading anchor with unchanged inset")
    else:
        # At 1280×900 this eight-row fixture fits; no Latest pill or scroll is needed.
        origin_y=emoji_body()["absolutePosition"]["y"]
        call("click_element",elementHandle=rich_element("Jump to replied message","Button")["handle"])
        call("click_element",elementHandle=rich_element("Return to message","Button")["handle"])
        assert abs(emoji_body()["absolutePosition"]["y"]-origin_y)<=1, "fit-history reply return moved content"
        print("PASS: rich fixture fits viewport; reply/Return preserve content without a Latest pill")
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
    rich_element("Saved", "Text")
    assert any(path.read_bytes() == b"Synthetic attachment\n" for path in (Path(tempfile.gettempdir()) / "line-desktop-demo-exports").glob("*") if path.is_file()), "demo export bytes differ"
    print("PASS: inline rich image, fallback/newline clipboard, reply jump/return, file duration metadata and synthetic Save")

if args.media:
    cases = [
        ("Media timeout", "Image took too long to load", True),
        ("Media network", "Couldn’t load this image", True),
        ("Media 404", "Image is no longer available", False),
        ("Media 403", "You don’t have access to this image", True),
        ("Media 500", "Image is temporarily unavailable", True),
        ("Media unsupported sender", "Not available for this sender", False),
        ("Media rejected path", "Image is unavailable", False),
        ("Media too large", "Image too large", False),
        ("Media decode failure", "Couldn’t display this image", False),
        ("Media cancelled", "Image loading cancelled", True),
    ]
    for scenario, reason, retriable in cases:
        demo_scenario("Media failures")
        call("click_element", elementHandle=scenario_action(scenario)["handle"])
        element(reason, "Text")
        retries = [item for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel") == "Retry image"]
        assert bool(retries) == retriable, (scenario, retries)
        assert not any("Refresh" in item.get("accessibleLabel", "") for item in tree())
        if retriable:
            call("click_element", elementHandle=retries[0]["handle"])
            assert not any(item.get("accessibleLabel") == reason for item in tree()), "Retry kept the cached failure"
    print("PASS: all media failure reasons, permanent classes without Retry, and per-item Retry completion")

if args.screenshot:
    result = rpc("tools/call", {"name": "take_screenshot", "arguments": {"windowHandle": window}})
    image = next(item for item in result["content"] if item["type"] == "image")
    args.screenshot.write_bytes(base64.b64decode(image["data"]))
print(f"PASS: MCP draft retention, chat selection, filtering, local demo Ctrl+Enter send, and multiline input; scale {properties.get('scaleFactor')}")

if args.unread:
    demo_scenario("Long history")
    wait_for(lambda: any("39: compact message" in item.get("accessibleValue", item.get("accessibleLabel", "")) for item in tree()), "long history page was not applied")
    def scroll_unread_history():
        call("dispatch_pointer_scroll",windowHandle=window,position={"x":600,"y":280},deltaY=700,deltaX=0)
        return any(item.get("accessibleLabel") == "Latest" for item in tree())
    wait_for(scroll_unread_history, "long history did not scroll away from Latest")
    candidates=[item for item in tree() if item.get("accessibleRole")=="TextInput" and any(base.get("id","").endswith("::body") for base in item.get("typeNamesAndIds",[])) and 120<item["absolutePosition"]["y"]<380]
    anchor=candidates[0] if candidates else None
    demo_scenario("Unread scenario")
    def unread_badge(label, count):
        return any(item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith(label + ", ") and f", {count} unread" in item["accessibleLabel"] for item in tree())
    wait_for(lambda: unread_badge("葵", 3), "open-chat arrivals lost unread before exposure")
    wait_for(lambda: unread_badge("Weekend plans", 2), "other-chat unread badge did not update")
    if anchor:
        after=next(item for item in tree() if item.get("accessibleRole")==anchor.get("accessibleRole") and item.get("accessibleValue")==anchor.get("accessibleValue"))
        assert abs(after["absolutePosition"]["y"]-anchor["absolutePosition"]["y"])<=1, f"unread arrivals moved content: {anchor['absolutePosition']} -> {after['absolutePosition']}"
    element("Latest · 3 new","Button")
    # The divider can be outside the bounded rendered range while scrolled up.
    call("click_element",elementHandle=element("Latest · 3 new","Button")["handle"])
    # A background automation window must stay unread. An already-active owner
    # may expose and read all three; focus is never requested by this test.
    current=element("葵","Button")
    assert not any(f", {n} unread" in current["accessibleLabel"] for n in [1,2]),"complete exposure left a partial boundary"
    element("Unread messages","Text")
    assert ", 2 unread" in element("Weekend plans","Button")["accessibleLabel"],"reading one chat cleared another chat"
    print("PASS: unread row badges, separate-chat counts, persistent divider and stable scrolled arrival anchor; no native focus request")


def scenario_key(text, **arguments):
    call("dispatch_key_event", windowHandle=window, text=text, **arguments)


def scenario_reverse_tab():
    scenario_key("\x10", eventType="Press")
    scenario_key("\t", eventType="PressAndRelease")
    scenario_key("\x10", eventType="Release")


for closing_key in ("\x1b", " "):
    scenario_key("\uf70f")
    scenario_key("\t", eventType="PressAndRelease")
    assert scenario_element("Long history").get("accessibleDescription") == "Focused", "Tab from Close did not focus History"
    scenario_reverse_tab()
    assert scenario_element("Close demo scenarios").get("accessibleDescription") == "Focused"
    scenario_reverse_tab()
    assert scenario_element(last_scenario_label).get("accessibleDescription") == "Focused", "reverse Tab focused the wrong scenario"
    scenario_key(closing_key, eventType="PressAndRelease")
    assert not call("find_elements_by_id", windowHandle=window, elementsId="DemoScenarios::scenario-panel").get("elementHandles", []), "keyboard activation did not dismiss scenarios"
    scenario_key("\uf70f")
    assert scenario_element("Close demo scenarios").get("accessibleDescription") == "Focused"
    scenario_key("\t", eventType="PressAndRelease")
    assert scenario_element("Long history").get("accessibleDescription") == "Focused", "stale last-scenario focus trapped Tab after reopening"
    scenario_key("\x1b")

scenario_key("\uf70f")
call("click_element", elementHandle=scenario_action("Media failures")["handle"])
scenario_reverse_tab()
assert scenario_element("Media cancelled").get("accessibleDescription") == "Focused", "reverse Tab did not focus the final media scenario"
scenario_key(" ", eventType="PressAndRelease")
scenario_key("\uf70f")
scenario_key("\t", eventType="PressAndRelease")
assert scenario_element("Long history").get("accessibleDescription") == "Focused", "stale media focus trapped Tab after reopening"
scenario_key("\x1b")
print("PASS: exact F12 reverse-Tab target and Escape/Space activation → reopen → Tab focus, including media")

if args.auth_recovery:
    # Failed sign-in stops the old stream. Check this terminal auth state last,
    # after other groups have finished using the demo connection.
    before = call("get_element_properties", elementHandle=composer).get("accessibleValue")
    demo_scenario("Expired sign-in")
    call("click_element", elementHandle=element("Sign in again", "Button")["handle"])
    call("click_element", elementHandle=element("Start", "Button")["handle"])
    deadline = time.monotonic() + 3
    while not any("LINE refused sign-in" in item.get("accessibleLabel", "") for item in tree()):
        assert time.monotonic() < deadline, "synthetic sign-in did not report its failure"
        time.sleep(.05)
    call("dispatch_key_event", windowHandle=window, text="\x1b")
    assert element("Sign in again", "Button").get("accessibleDescription") == "Focused"
    assert call("get_element_properties", elementHandle=composer).get("accessibleValue") == before, "failed sign-in changed the draft"
    print("PASS: synthetic sign-in failure preserves cached composer and returns focus to recovery")
