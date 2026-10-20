#!/usr/bin/env python3
"""Run behavioural UI checks in owned offline headless demos; never builds."""
import argparse
import base64
import datetime
import json
from pathlib import Path
import random
import secrets
import socket
import sys
import tempfile
import time
import traceback
import urllib.error
import urllib.request

from run_ui_debug import UI_TEMP, launch_demo

ROOT = Path(__file__).resolve().parent.parent
counter = 0
connected = False


class WrongInstanceError(RuntimeError):
    pass


def rpc(method, params, timeout=10):
    global counter
    counter += 1
    request = urllib.request.Request(
        f"http://127.0.0.1:{args.port}/mcp",
        data=json.dumps({"jsonrpc": "2.0", "id": counter, "method": method, "params": params}).encode(),
        headers={"Content-Type": "application/json", "Accept": "application/json, text/event-stream"},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            result = json.load(response)
    except (urllib.error.URLError, TimeoutError, ConnectionError) as error:
        if connected:
            raise ConnectionError("Demo MCP connection lost: app crashed or stopped") from error
        raise
    if "error" in result:
        raise RuntimeError("MCP request failed")
    return result["result"]

def call(name, **arguments):
    result = rpc("tools/call", {"name": name, "arguments": arguments})
    if result.get("isError"):
        message = str(result.get("content"))
        if "Invalid element handle" in message or "refers to element that was destroyed" in message or "Attempting to access deleted window" in message:
            raise LookupError(message)
        raise RuntimeError(f"MCP tool failed: {name}: {message}")
    parsed = json.loads(result["content"][0]["text"])
    if name == "get_element_tree":
        assert not any("Internal error:" in str(item.get(field, "")) for item in parsed["elements"] for field in ("accessibleLabel", "accessibleValue", "accessibleDescription")), "debug internal-error toast appeared"
    return parsed

def tree(handle=None):
    result = call("get_element_tree", elementHandle=root if handle is None else handle, maxElements=2500)
    assert not result.get("truncated"), "UI tree exceeded test bound"
    return result["elements"]

def focused_control():
    marker = bounds_by_id("DemoExtension::focus-marker")
    call("invoke_accessibility_action", elementHandle=marker["handle"], action="Default_")
    return json.loads(call("get_element_properties", elementHandle=marker["handle"])["accessibleValue"])


def is_focused(item, focus=None):
    focus = focused_control() if focus is None else focus
    return (item.get("accessibleLabel") == focus.get("label")
            and item.get("accessibleRole") == focus.get("role")
            and all(abs(item["absolutePosition"].get(axis, 0) - focus[axis]) < .01 for axis in ("x", "y"))
            and all(abs(item["size"][extent] - focus[extent]) < .01 for extent in ("width", "height")))


def element(label, role, read_tree=tree):
    item = next((item for item in read_tree() if (item.get("accessibleLabel") == label or item.get("accessibleLabel", "").startswith(label + ", ")) and item.get("accessibleRole") == role), None)
    if item is None:
        raise LookupError(f"Missing {role} {label!r}")
    return item

def wait_for(predicate, message, timeout=5):
    # MCP input dispatch completes before worker deltas and layout callbacks.
    # Poll the observable outcome instead of assuming one RPC or a sleep drains them.
    deadline = time.monotonic() + timeout
    while True:
        try:
            result = predicate()
        except LookupError:
            result = None
        if result:
            return result
        assert time.monotonic() < deadline, message
        time.sleep(.05)


def wait_element(label, role="Button"):
    return wait_for(lambda: element(label, role), f"Missing {role} {label!r}")

def bounds_by_id(qualified_id):
    handles = call("find_elements_by_id", windowHandle=window, elementsId=qualified_id)["elementHandles"]
    assert len(handles) == 1, f"non-unique geometry: {qualified_id}"
    return {"handle": handles[0], **call("get_element_properties", elementHandle=handles[0])}

def scenario_element(label):
    handles = call("find_elements_by_id", windowHandle=window, elementsId="DemoScenarios::scenario-panel").get("elementHandles", [])
    if not handles:
        raise LookupError("demo scenario dialog did not open")
    assert len(handles) == 1, "demo dialog was not unique"
    return element(label, "Button", lambda: tree(handles[0]))


def scenario_action(label):
    # Scrolling explores clipped actions; wait_for bounds worker/layout completion.
    deltas = iter((10000, *([-320] * 12)))
    def find_action():
        try:
            return scenario_element(label)
        except LookupError:
            close = scenario_element("Close demo scenarios")
            position = {"x": close["absolutePosition"]["x"] + close["size"]["width"] / 2,
                        "y": close["absolutePosition"]["y"] + close["size"]["height"] + 60}
            delta = next(deltas, 0)
            call("dispatch_pointer_scroll", windowHandle=window, position=position, deltaY=delta)
            return None
    return wait_for(find_action, f"demo scenario was not reachable: {label}")


def demo_scenario(label):
    global scenario_bounds
    started = datetime.datetime.now()
    key("\uf70f")  # F12
    call("click_element", elementHandle=scenario_action(label)["handle"])
    scenario_bounds = (started, datetime.datetime.now())


def show_chats():
    if properties["size"]["width"] / properties.get("scaleFactor", 1) < 740:
        back = next((item for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel") == "Back"), None)
        if back: call("click_element", elementHandle=back["handle"])
    ctrl("2")


def handle(label, role):
    return wait_element(label, role)["handle"]


def click(label, role="Button", **arguments):
    def activate():
        selected = element(label, role)["handle"]
        call("click_element", elementHandle=selected, **arguments)
        return True
    wait_for(activate, f"Could not click {role} {label!r}")


def key(text, event_type="PressAndRelease"):
    call("dispatch_key_event", windowHandle=window, text=text, eventType=event_type)


def ctrl(text):
    key("\x11", "Press")
    key(text)
    key("\x11", "Release")


def set_value(label, value, role="TextInput"):
    def apply():
        selected = element(label, role)["handle"]
        call("set_element_value", elementHandle=selected, value=value)
        return True
    wait_for(apply, f"Could not set {role} {label!r}")


def bodies():
    return [item for item in tree() if item.get("accessibleRole") == "TextInput" and any(base.get("id", "").endswith("::body") for base in item.get("typeNamesAndIds", []))]


def screenshot(name):
    if args.screenshots:
        args.screenshots.mkdir(parents=True, exist_ok=True)
        result = rpc("tools/call", {"name": "take_screenshot", "arguments": {"windowHandle": window}})
        picture = next(item for item in result["content"] if item["type"] == "image")
        (args.screenshots / f"{current_group}-{name}.png").write_bytes(base64.b64decode(picture["data"]))

def smoke_chat(label):
    show_chats()
    click(label, "Button")
    def selected_chat_visible():
        items = tree()
        if properties["size"]["width"] / properties.get("scaleFactor", 1) < 740:
            return any(item.get("accessibleRole") == "Button" and item.get("accessibleLabel") == label for item in items) and any(item.get("accessibleLabel") == "Message draft" for item in items)
        return any(item.get("accessibleItemSelected") and item.get("accessibleLabel", "").startswith(label + ", ") for item in items)
    wait_for(selected_chat_visible, f"chat selection did not settle: {label}")

def assert_restored_focus(label, role="Button"):
    focus = focused_control()
    item = element(label, role)
    assert is_focused(item, focus), f"focus not restored to {label}: {focus}"
    # Hidden ancestors remove their descendants from the accessible tree.
    assert focus["visible"], f"restored focus is hidden: {label}"
    size = call("get_window_properties", windowHandle=window)
    scale = size.get("scaleFactor", 1)
    for axis, extent in (("x", "width"), ("y", "height")):
        assert item["size"][extent] > 0, f"restored focus has no area: {label}"
        assert 0 <= item["absolutePosition"][axis] < size["size"][extent] / scale, f"restored focus is outside the window: {label}"


def check_detail_stack():
    def focused(label, role="Button"):
        assert_restored_focus(label, role)
    click("Friends")
    search=element("Search friends and groups", "TextInput")
    call("set_element_value",elementHandle=search["handle"],value="")
    click("Open profile: Weekend plans")
    wait_element("Group info", "Text")
    call("click_element", elementHandle=element("Group info", "Text")["handle"], button="Right")
    assert not any(item.get("accessibleLabel") == "Copy name" for item in tree()), "right-click on Detail content opened the profile menu"
    assert any(item.get("accessibleLabel")=="Weekend plans" for item in tree()), "group info is not bound to its chat"
    click("Members (25)")
    wait_element("Search members", "TextInput")
    call("hover_element", elementHandle=element("Search members", "TextInput")["handle"])
    assert all(call("get_element_properties", elementHandle=item["handle"]).get("computedOpacity", 0) == 0 for item in tree() if item.get("accessibleLabel", "").startswith("Chat with ")), "pointer users saw a keyboard-selected row's Chat with action"
    set_value("Search members", "葵")
    click("Search members", "TextInput")
    key("\uf701")
    focused("Group members", "List")
    focus_tab()
    focused("Chat with 葵")
    assert call("get_element_properties", elementHandle=element("Chat with 葵", "Button")["handle"]).get("computedOpacity", 0) == 1, "keyboard-selected Chat with action is hidden"
    focus_tab(True)
    focused("Group members", "List")
    click("Open member: 葵")
    wait_element("Profile", "Text")
    click("Rename")
    assert_restored_focus("Name", "TextInput")
    original_name = element("Name", "TextInput")["accessibleValue"]
    key("x")
    assert element("Name", "TextInput")["accessibleValue"] == "x" + original_name, "rename opening selected the name or moved its caret"
    set_value("Name", "Aoi test 🌿")
    click("Save")
    wait_element("Aoi test 🌿", "Text")
    wait_element("葵", "Text")
    click("Rename")
    set_value("Name", "葵")
    click("Save")
    wait_element("葵", "Text")
    click("Open profile photo")
    wait_element("Photo in viewer", "Image")
    key("\x1b")
    focused("Open profile photo")
    click("Back")
    focused("Group members", "List")
    set_value("Search members", "蓮")
    click("Open member: 蓮")
    click("Add friend")
    wait_element("Rename", "Button")
    wait_element("Chat", "Button")
    click("Back")
    set_value("Search members", "Unknown member")
    click("Open member: Unknown member")
    wait_element("Profile", "Text")
    wait_for(lambda: not any(item.get("accessibleLabel") == "Open member: Unknown member" for item in tree()), "unknown member profile did not replace the member list")
    wait_element("Unknown member", "Text")
    assert not any(item.get("accessibleLabel") in ("Chat", "Rename", "Add friend") for item in tree()), "unknown profile exposed a write"
    click("Back")
    set_value("Search members", "葵")
    click("Back")
    focused("Members (25)")
    click("Members (25)")
    set_value("Search members", "葵")
    click("Open member: 葵")
    key("\x1b")
    focused("Group members", "List")
    key("\x1b")
    focused("Members (25)")
    click("Members (25)")
    set_value("Search members", "葵")
    click("Open member: 葵")
    click("Close details")
    assert not any(item.get("accessibleLabel")=="Close details" for item in tree()), "close did not clear the stack"
    focused("Friends and groups", "List")
    click("Open profile: Weekend plans")
    click("Notifications", "Checkbox")
    wait_for(lambda: element("Notifications", "Checkbox").get("accessibleValue") == "Off", "mute did not update", timeout=3)
    click("Leave group")
    wait_element("Leave group?", "Text")
    key("\x1b")
    click("Leave group")
    click("Leave")
    click("Close details")
    click("Chats")
    smoke_chat("葵")

def check_idle():
    def switches():
        status = Path(f"/proc/{demo_pid}/task/{demo_pid}/status").read_text()
        return int(next(line.split()[1] for line in status.splitlines()
                        if line.startswith("voluntary_ctxt_switches:")))

    def measure(label):
        time.sleep(2)  # Let scrolling, transient UI and worker activity settle.
        before = switches()
        started = time.monotonic()
        time.sleep(2)
        rate = (switches() - before) / (time.monotonic() - started)
        print(f"IDLE {label}: {rate:.1f} main-thread voluntary switches/s", flush=True)
        assert rate < 50, f"{label}: main thread woke {rate:.1f}/s while idle (limit 50/s)"

    show_chats()
    measure("chat list")
    click("葵", "Button")
    measure("open chat")


def check_smoke():
    global draft, before_shortcut, window, root, properties
    show_chats()
    wait_for(lambda: "葵 changed the group photo" in element("Weekend plans", "Button")["accessibleLabel"], "demo system preview did not resolve its known actor")
    draft = "日本語の下書き\nDraft with emoji 🐈"
    smoke_chat("葵")
    call("set_element_value", elementHandle=handle("Message draft", "TextInput"), value=draft)
    smoke_chat("Weekend plans")
    call("set_element_value", elementHandle=handle("Message draft", "TextInput"), value="Another draft")
    smoke_chat("葵")
    value = wait_element("Message draft", "TextInput")
    assert value.get("accessibleValue") == draft, "draft did not survive chat switching"
    show_chats()
    call("set_element_value", elementHandle=handle("Find a conversation", "TextInput"), value="Weekend")
    chat_rows = [item for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith(("葵, ", "Weekend plans, ", "Notes to self, "))]
    assert len(chat_rows) == 1 and chat_rows[0]["accessibleLabel"].startswith("Weekend plans, "), "search did not filter chats"
    call("set_element_value", elementHandle=handle("Find a conversation", "TextInput"), value="")
    smoke_chat("葵")
    assert element("Send", "Button").get("accessibleEnabled", False), "demo transport did not enable Send"
    call("click_element", elementHandle=handle("Message draft", "TextInput"))
    key("\n")
    value = wait_element("Message draft", "TextInput")
    assert value.get("accessibleValue", "").endswith("\n"), "Enter did not insert a newline"
    before_shortcut = value.get("accessibleValue")
    ctrl("\n")
    wait_for(lambda: not (wait_element("Message draft", "TextInput").get("accessibleValue", "")), "durable send acknowledgement did not clear the draft", timeout=2)
    assert wait_element("Message draft", "TextInput").get("accessibleValue", "") == "", "Ctrl+Enter did not clear the submitted draft"
    assert any(item.get("accessibleValue") == before_shortcut for item in tree()), "submitted draft did not appear in history"
    wait_element("Sent", "Image")
    call("set_element_value", elementHandle=handle("Message draft", "TextInput"), value=before_shortcut)

def check_emoji():
    show_chats()
    smoke_chat("Emoji 🌸 😀")
    sample = "🌸 😀 ❤️ ❤︎ 👩‍💻 🇯🇵 1️⃣"
    set_value("Message draft", sample)
    click("Send")
    wait_for(lambda: not element("Message draft", "TextInput").get("accessibleValue"), "emoji send did not finish")
    wait_for(lambda: any(item.get("accessibleValue") == sample for item in bodies()), "emoji message was not sent")
    set_value("Message draft", sample)
    click("Message draft", "TextInput")
    screenshot("emoji-surfaces")


def check_signed_out():
    for section in ("Chats", "Friends"):
        click(section)
        wait_element("Not signed in", "Text")
        assert not any(item.get("accessibleLabel") in ("Your profile", "Your avatar", "No friends yet", "No conversations yet", "Choose a conversation", "Message draft") for item in tree())
        screenshot(section.lower())
        click("Sign in")
        screenshot(section.lower() + "-before-start")
        assert not any(item.get("accessibleRole") == "Image" and "QR code" in item.get("accessibleLabel", "") for item in tree())
        click("Start")
        wait_for(lambda: element("Scan this QR code with LINE on your phone", "Image"), "QR did not arrive after Start")
        screenshot(section.lower() + "-qr")
        click("Back")
        click("Cancel sign-in")
        wait_for(lambda: any(item.get("accessibleLabel") == "Not signed in" for item in tree()), "Back did not cancel pairing")
    click("Chats")
    key("\uf70f")
    wait_element("Synthetic interaction scenarios", "Text")
    call("click_element", elementHandle=scenario_action("Sign-in keyring failure")["handle"])
    click("Sign in")
    click("Start")
    wait_element("Unlock your keyring (KWallet/GNOME Keyring) and try again. Ensure Secret Service is running with a default keyring.", "Text")
    assert not any("Local data could not be saved" in item.get("accessibleLabel", "") for item in tree())
    click("Back")
    click("Settings")
    screenshot("settings")
    for section in ("Notifications", "Background"):
        click(section, "Tab")
        screenshot("settings-" + section.lower())
    click("About", "Tab")
    assert not any(item.get("accessibleRole") == "Button" and item.get("accessibleLabel") in ("", "Offline preview") for item in tree())
    screenshot("settings-about")


def check_missing_module():
    click("Sign in")
    message = "Can't load the LINE crypto module at /demo/crypto.wasm. Check LINE for Chrome or the developer override."
    wait_element(message, "Text")
    wait_element("Sign-in needs a supported LINE for Chrome installation.", "Text")
    for _ in range(2):
        click("Check again")
        wait_element(message, "Text")
        wait_element("Check again", "Button")
        assert not any(item.get("accessibleRole") == "Image" and "QR code" in item.get("accessibleLabel", "") for item in tree())
    click("Back")
    wait_element("Not signed in", "Text")


def focus_label():
    current = focused_control()
    focused = [item for item in tree() if is_focused(item, current)]
    assert len(focused) == 1, f"unlabelled or ambiguous Tab stop: {focused}"
    label = focused[0].get("accessibleLabel", "")
    assert label, "focused control has no label"
    assert focused[0]["size"]["width"] > 0 and focused[0]["size"]["height"] > 0, f"hidden focus: {label}"
    return label

def focus_tab(reverse=False):
    if reverse: key("\x10", "Press")
    key("\t")
    if reverse: key("\x10", "Release")

def check_chain(surface, expected, seed):
    assert len(expected) == len(set(expected)), f"{surface}: duplicate expected stop"
    if seed[1] == "TextInput":
        click(*seed)
    for _ in expected:
        if focus_label() == seed[0]: break
        focus_tab()
    assert focus_label() == seed[0], f"{surface}: seed did not focus {seed[0]} (at {focus_label()})"
    # One full cycle in each direction checks every focus edge and wrap.
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


def check_narrow_chats_focus():
    show_chats()
    click("Friends")
    set_value("Search friends and groups", "Weekend")
    click("Open profile: Weekend plans")
    key("\x1b")
    assert_restored_focus("Message draft", "TextInput")
    show_chats()
    for recovery in ([], ["Sign in again"]):
        if recovery: demo_scenario("Expired sign-in")
        # Tab follows visual order
        expected = ["Navigation", "New chat", "Find a conversation", "All"] + recovery + ["Conversations"]
        check_chain("narrow chats", expected, ("Find a conversation", "TextInput"))
        set_value("Find a conversation", "葵")
        check_chain("narrow chats search clear", expected[:expected.index("Find a conversation") + 1] + ["Clear search"] + expected[expected.index("Find a conversation") + 1:], ("Find a conversation", "TextInput"))
        click("Clear search")


def check_focus():
    logical_width = properties["size"]["width"] / properties.get("scaleFactor", 1)
    narrow = logical_width < 740
    panel = logical_width >= 1060
    layer_kind = "panel" if panel else "sheet"

    def browser():
        if narrow:
            key("\x12", "Press")  # Alt+Left also works from layer openers.
            key("\uf702")
            key("\x12", "Release")

    def browser_stops(search_label, list_label, recovery):
        # Tab follows visual order
        return ["Navigation"] + (["New chat"] if list_label == "Conversations" else []) + [search_label] + (["All"] if list_label == "Conversations" else []) + (recovery if narrow else []) + [list_label]

    if panel:
        smoke_chat("葵")
        title = next(item for item in tree() if item.get("accessibleLabel") == "葵" and item.get("accessibleRole") == "Button")
        call("click_element", elementHandle=title["handle"])
        body = next(item for item in reversed(tree()) if item.get("accessibleRole") in ("Text", "TextInput") and item.get("accessibleLabel", "").startswith(("Incoming from ", "Outgoing, ")))
        call("click_element", elementHandle=body["handle"])
        click("More")
        click("Profile", "Text")
        key("\x1b")
        assert_restored_focus("Message draft", "TextInput")

    # Tab follows visual order
    click("Chats")
    click("Find a conversation", "TextInput")
    focus_tab(True)
    assert focus_label() == "New chat"
    for arrow in ("\uf703", "\uf702"):
        key(arrow)
        focus_tab()
        assert focus_label() == "Find a conversation", "New chat arrow moved focus"
        focus_tab()
        assert focus_label() == "All", "New chat arrow moved the chip cursor"
        focus_tab(True)
        focus_tab(True)
        assert focus_label() == "New chat"

    # Chips are one Tab stop; arrows move focus without activating a filter.
    click("Chats")
    click("Find a conversation", "TextInput")
    focus_tab()
    assert focus_label() == "All"
    key("\uf703")
    assert focus_label() == "Friends"
    key(" ")
    assert focus_label() == "Friends"
    assert not any(item.get("accessibleLabel", "").startswith("Weekend plans, ") for item in tree()), "Space did not activate Friends filter"
    key("\uf703")
    assert focus_label() == "Groups"
    key("\n")
    wait_element("Weekend plans", "Button")
    assert not any(item.get("accessibleLabel", "").startswith("葵, ") for item in tree()), "Enter did not activate Groups filter"
    key("\uf702")
    key("\uf702")
    key("\n")
    assert focus_label() == "All"
    # Category headers are arrow-reachable list rows, with both activation keys.
    click("Friends")
    click("Search friends and groups", "TextInput")
    focus_tab()
    assert focus_label() == "Friends and groups"
    key("\uf729")  # Home
    key(" ")
    assert not any(item.get("accessibleLabel", "").startswith("Open profile: Weekend plans") for item in tree()), "Space did not collapse Groups"
    key("\uf701")  # Next row is now the Friends category.
    key("\n")
    assert not any(item.get("accessibleLabel", "").startswith("Open profile:") for item in tree()), "Enter did not collapse Friends"
    key(" ")
    key("\uf700")
    key("\n")
    wait_element("Open profile: 葵", "Button")
    wait_element("Open profile: Weekend plans", "Button")
    def check_focus_shell(offline):
        click("Chats")
        click("葵")
        browser()
        if offline: demo_scenario("Expired sign-in")
        suffix = "with strip" if offline else "without strip"
        recovery = ["Sign in again"] if offline else []
        for surface, search_label, list_label in [("chats", "Find a conversation", "Conversations"), ("friends", "Search friends and groups", "Friends and groups")]:
            click("Friends" if surface == "friends" else "Chats")
            expected = browser_stops(search_label, list_label, recovery)
            if not narrow:
                expected += ["葵", "More"] + recovery + ["Message history"]
                if element("Stickers", "Button").get("accessibleEnabled"): expected.append("Stickers")
                if element("Attach", "Button").get("accessibleEnabled"): expected.append("Attach")
                # Tab follows the composer's visual order
                expected.append("Message draft")
                if element("Send", "Button").get("accessibleEnabled"): expected.append("Send")
            check_chain(f"{surface} {suffix}", expected, (search_label, "TextInput"))
            set_value(search_label, "葵")
            check_chain(f"{surface} search clear {suffix}", expected[:expected.index(search_label) + 1] + ["Clear search"] + expected[expected.index(search_label) + 1:], (search_label, "TextInput"))
            click("Clear search")
        click("Chats")
        for chat in ("葵", "Weekend plans"):
            click(chat)
            set_value("Message draft", "Focus check")
            # Tab follows visual order
            expected = ["Back"] if narrow else ["Navigation", "New chat", "Find a conversation", "All", "Conversations"]
            expected.append(chat)
            if chat == "Weekend plans": expected.append(element("Members", "Button")["accessibleLabel"])
            expected += ["More"] + recovery + ["Message history"]
            if element("Stickers", "Button").get("accessibleEnabled"): expected.append("Stickers")
            if element("Attach", "Button").get("accessibleEnabled"): expected.append("Attach")
            # Tab follows the composer's visual order
            expected.append("Message draft")
            if element("Send", "Button").get("accessibleEnabled"): expected.append("Send")
            check_chain(f"conversation {chat} {suffix}", expected, ("Message draft", "TextInput"))
            if chat == "Weekend plans":
                click("Members")
                layer = ["Close details", "Search members", "Group members"]
                # Tab follows visual order
                prefix = expected if panel else [] if narrow else ["Navigation", "New chat", "Find a conversation", "All", "Conversations"]
                check_chain(f"members {layer_kind} {suffix}", prefix + layer, ("Close details", "Button"))
                set_value("Search members", "葵")
                check_chain(f"members search clear {suffix}", prefix + layer[:2] + ["Clear search"] + layer[2:] + ["Chat with 葵"], ("Close details", "Button"))
                click("Open member: 葵")
                check_chain(f"member profile {layer_kind} {suffix}", prefix + ["Back", "Profile more", "Close details", "Open profile photo", "Chat", "Rename"], ("Close details", "Button"))
                key("\x1b")
                set_value("Search members", "蓮")
                click("Open member: 蓮")
                check_chain(f"member profile without chat {layer_kind} {suffix}", prefix + ["Back", "Profile more", "Close details", "Open profile photo", "Add friend"], ("Close details", "Button"))
                key("\x1b")
                key("\x1b")
            browser()
        click("Friends")
        click("Open profile: 葵")
        prefix = [] if narrow else ["Navigation", "Search friends and groups", "Friends and groups"]
        if panel:
            prefix += ["Weekend plans", element("Members", "Button")["accessibleLabel"], "More"] + recovery + ["Message history"]
            if element("Stickers", "Button").get("accessibleEnabled"): prefix.append("Stickers")
            if element("Attach", "Button").get("accessibleEnabled"): prefix.append("Attach")
            # Tab follows the composer's visual order
            prefix.append("Message draft")
            if element("Send", "Button").get("accessibleEnabled"): prefix.append("Send")
        check_chain(f"profile {layer_kind} {suffix}", prefix + ["Profile more", "Close details", "Open profile photo", "Chat", "Rename"], ("Close details", "Button"))
        key("\x1b")
        click("Settings")
        check_chain(f"settings {suffix}", ["Close settings", "Settings section", "Message text size", "Enter sends"], ("Close settings", "Button"))
        key("\x1b")
        click("Chats")
    check_focus_shell(False)
    demo_scenario("Reconnect blip")
    wait_for(lambda: not any(item.get("accessibleRole") == "Banner" for item in tree()), "reconnect did not finish", 7)
    # Leave a connected Chats browser with empty drafts and queries for later checks.
    for chat in ("Weekend plans", "葵"):
        show_chats()
        click(chat)
        set_value("Message draft", "")
        if chat == "Weekend plans":
            click("Members")
            set_value("Search members", "")
            key("\x1b")
    show_chats()
    set_value("Find a conversation", "")
    wait_for(lambda: not any(item.get("accessibleRole") == "Banner" for item in tree()), "focus check did not restore the connected shell", timeout=7)
    assert focus_label() == "Find a conversation"

    check_focus_shell(True)


def assert_reactor_popup_visible():
    popup = wait_element("Who reacted", "List")
    assert popup["size"]["width"] > 0 and popup["size"]["height"] > 0, "reactor list is hidden"
    geometry = focused_control()["popup"]
    assert geometry is not None, "reactor popup is not inside the window"
    bounds = call("get_window_properties", windowHandle=window)
    scale = bounds.get("scaleFactor", 1)
    for axis, extent in (("x", "width"), ("y", "height")):
        start = geometry[axis] + popup["absolutePosition"].get(axis, 0)
        assert 0 <= start and start + popup["size"][extent] <= bounds["size"][extent] / scale, "reactor popup is outside the window"


def check_reactor_popup(label):
    chip = wait_element(label, "Button")
    assert chip.get("accessibleItemSelected"), "own chip is not highlighted"
    click(label)
    wait_element("You", "Text")
    assert_reactor_popup_visible()
    screenshot("reactors")
    key("\x1b")
    assert not any(r.get("accessibleLabel") == "You" and r.get("accessibleRole") == "Text" for r in tree()), "Escape left reactor popup open"
    assert is_focused(element(label, "Button")), "Escape did not return focus to chip"
    for activation in ("\n", " "):
        key(activation)
        wait_element("You", "Text")
        key("\x1b")
        assert not any(r.get("accessibleLabel") == "You" and r.get("accessibleRole") == "Text" for r in tree()), "reactor popup did not close between activations"
        assert is_focused(element(label, "Button"))
    assert element(label, "Button").get("accessibleItemSelected"), "opening reactors toggled own reaction"


def check_pack_switch():
    click("Happy shapes", "Tab")
    wait_for(lambda: element("Happy shapes", "Tab").get("accessibleItemSelected"), "pack selection did not switch grid")
    wait_element("Send sticker 1", "Button")
    wait_element("Happy shapes stickers", "List")
    screenshot("switched-pack")


def check_pack_wheel():
    strip = bounds_by_id("StickerPicker::strip")
    call("scroll_element", elementHandle=strip["handle"], deltaX=0, deltaY=1000)
    first = wait_element("Little shapes", "Tab")
    assert first["absolutePosition"]["x"] >= strip["absolutePosition"]["x"], "vertical wheel did not reveal earlier packs"
    call("scroll_element", elementHandle=strip["handle"], deltaX=0, deltaY=-1000)
    last = wait_element("Shapes 12", "Tab")
    assert last["absolutePosition"]["x"] + last["size"]["width"] <= strip["absolutePosition"]["x"] + strip["size"]["width"], "vertical wheel did not reveal later packs"
    call("scroll_element", elementHandle=strip["handle"], deltaX=0, deltaY=1000)



def check_composer_growth():
    click("Settings")
    click("Message text size", "Combobox")
    click("Large")
    key("\x1b")
    set_value("Message draft", "")
    minimum = bounds_by_id("Composer::composer")["size"]["height"]
    set_value("Message draft", "One line")
    assert abs(bounds_by_id("Composer::composer")["size"]["height"] - minimum) <= 1, "single-line draft grew beyond its minimum"
    set_value("Message draft", "First line\nSecond line\nThird line")
    assert bounds_by_id("Composer::composer")["size"]["height"] > minimum + 10, "multiline draft did not grow at large text size"
    click("Settings")
    click("Message text size", "Combobox")
    click("Default")
    key("\x1b")
    set_value("Message draft", draft)


def check_composer_alignment():
    for text, name in (("One line", "one-line"), ("Line one\nLine two\nLine three", "multiline")):
        set_value("Message draft", text)
        field = bounds_by_id("Composer::composer")
        buttons = [element(label, "Button") for label in ("Stickers", "Attach", "Send")]
        assert all(b["size"] == {"width": 32, "height": 32} for b in buttons), "composer buttons must stay 32×32"
        centers = [b["absolutePosition"]["y"] + b["size"]["height"] / 2 for b in buttons]
        assert max(centers) - min(centers) <= 1, "composer buttons are not centred together"
        if name == "one-line":
            assert abs(centers[0] - field["absolutePosition"]["y"] - field["size"]["height"] / 2) <= 1, "buttons are not centred on the single-line field"
            bottom_insets = [field["absolutePosition"]["y"] + field["size"]["height"] - b["absolutePosition"]["y"] - b["size"]["height"] for b in buttons]
        else:
            insets = [field["absolutePosition"]["y"] + field["size"]["height"] - b["absolutePosition"]["y"] - b["size"]["height"] for b in buttons]
            assert all(abs(a - b) <= 1 for a, b in zip(insets, bottom_insets)), "buttons did not stay aligned with the bottom of the growing field"
        screenshot(name)


def check_compose():
    check_composer_growth()
    check_composer_alignment()
    set_value("Message draft", before_shortcut)
    click("Send")
    wait_for(lambda: not element("Message draft", "TextInput").get("accessibleValue"), "compose fixture send did not finish")
    wait_for(lambda: any(item.get("accessibleValue") == before_shortcut for item in bodies()), "compose fixture did not appear in history")


    # Select the just-sent message; its reaction target is visible at the tail.
    def open_reactions():
        body = next(item for item in reversed(tree()) if item.get("accessibleRole") == "TextInput" and item.get("accessibleValue") == before_shortcut and item.get("accessibleLabel", "").startswith("Outgoing,"))
        call("hover_element", elementHandle=body["handle"])
        reaction = min((item for item in tree() if item.get("accessibleLabel") == "React" and item.get("accessibleRole") == "Button"), key=lambda item: abs(item["absolutePosition"]["y"] - body["absolutePosition"]["y"]))
        position = reaction["absolutePosition"]
        call("move_pointer", windowHandle=window, position={"x": position["x"] - 2, "y": position["y"] + reaction["size"]["height"] / 2})
        assert any(item.get("accessibleLabel") == "React" for item in tree()), "React disappeared in the gap"
        call("hover_element", elementHandle=reaction["handle"])
        assert any(item.get("accessibleLabel") == "React" for item in tree()), "React disappeared over its button"
        call("click_element", elementHandle=reaction["handle"])
    open_reactions()
    wait_element("LOVE", "Button")
    click("LOVE")
    check_reactor_popup("LOVE, 1 reactions")
    open_reactions()
    wait_for(lambda: element("LOVE", "Button").get("accessibleItemSelected"), "own reaction was not highlighted")
    screenshot("reactions")
    key("\x1b")
    body = next(item for item in tree() if item.get("accessibleRole") == "TextInput" and item.get("accessibleValue") == before_shortcut)
    call("click_element", elementHandle=body["handle"])
    call("hover_element", elementHandle=element("Find a conversation", "TextInput")["handle"])
    for label in ("LOVE, 1 reactions", "React", "Reply"):
        focus_tab()
        assert is_focused(element(label, "Button")), f"reaction message Tab sequence missed {label}: {focused_control()}"
    for label in ("React", "LOVE, 1 reactions"):
        focus_tab(True)
        assert is_focused(element(label, "Button")), f"reaction message reverse Tab sequence missed {label}"
    focus_tab(True)
    assert is_focused(body), "reverse Tab from reaction chip skipped its message body"
    for label in ("LOVE, 1 reactions", "React", "Reply"):
        focus_tab()
        assert is_focused(element(label, "Button")), f"reaction message Tab sequence missed {label}: {focused_control()}"
    composer_start = next((label, "Button") for label in ("Stickers", "Attach") if element(label, "Button").get("accessibleEnabled"))
    focus_tab()
    assert is_focused(element(*composer_start)), "reaction message Reply skipped the composer's first control"
    open_reactions()
    click("LOVE")
    open_reactions()
    wait_for(lambda: not element("LOVE", "Button").get("accessibleItemSelected"), "repeat pick did not cancel")
    key("\x1b")
    body = next(item for item in tree() if item.get("accessibleRole") == "TextInput" and item.get("accessibleValue") == before_shortcut)
    call("click_element", elementHandle=body["handle"])
    for label in ("React", "Reply"):
        key("\t")
        assert any(item.get("accessibleLabel") == label and is_focused(item) for item in tree()), f"body → React → Reply lost focus at {label}"
    focus_tab(True)
    assert is_focused(element("React", "Button")), "reverse Tab from Reply skipped React"
    focus_tab(True)
    assert is_focused(body), "reverse Tab from React skipped its message body"
    for label in ("React", "Reply"):
        focus_tab()
        assert is_focused(element(label, "Button")), f"forward Tab skipped {label}"
    key("\t")
    # Tab follows the composer's visual order
    composer_start = next((label, "Button") for label in ("Stickers", "Attach") if element(label, "Button").get("accessibleEnabled"))
    assert is_focused(element(*composer_start)), "Reply did not tab to the first enabled composer control"
    def reply_via_menu():
        body = next(item for item in reversed(tree()) if item.get("accessibleRole") == "TextInput" and item.get("accessibleValue") == before_shortcut and item.get("accessibleLabel", "").startswith("Outgoing,"))
        call("click_element", elementHandle=body["handle"], button="Right")
        click("Reply", "Text")
        wait_element("Replying to You", "Text")
        wait_element("Cancel reply", "Button")
    reply_via_menu()
    screenshot("composer-reply")
    click("Cancel reply")
    wait_for(lambda: not any(item.get("accessibleLabel") == "Cancel reply" for item in tree()), "close did not cancel reply")
    assert is_focused(element("Message draft", "TextInput"))
    reply_via_menu()
    key("\x1b")
    wait_for(lambda: not any(item.get("accessibleLabel") == "Cancel reply" for item in tree()), "Escape did not cancel reply")
    reply_via_menu()
    set_value("Message draft", "Reply sent with original attached")
    click("Send")
    wait_for(lambda: any(item.get("accessibleValue") == "Reply sent with original attached" for item in tree()), "reply send missing")
    wait_for(lambda: not any(item.get("accessibleLabel") == "Cancel reply" for item in tree()), "reply bar remained after send")
    def new_message_has_quote():
        delegates = [item["handle"] for item in tree() if any(part.get("typeName") == "Message" for part in item.get("typeNamesAndIds", []))]
        for handle in delegates:
            rows = call("get_element_tree", elementHandle=handle, maxElements=200)["elements"]
            if any(item.get("accessibleValue") == "Reply sent with original attached" for item in rows):
                return any(item.get("accessibleLabel") == "Jump to replied message" and item.get("accessibleDescription") == " ".join(before_shortcut.split()) for item in rows)
        return False
    wait_for(new_message_has_quote, "new message itself did not show its original quote")
    screenshot("sent-reply")
    set_value("Message draft", before_shortcut)
    click("Stickers")
    wait_element("Send sticker 1", "Button")
    wait_for(lambda: element("Sticker preview 1", "Image").get("accessibleDescription") == "Loaded", "picker images did not load")
    screenshot("stickers")
    wait_for(lambda: len([item for item in tree() if item.get("accessibleRole") == "Tab"]) >= 2, "owned packages missing")
    check_pack_switch()
    key("\uF703")
    wait_for(lambda: element("Shapes 3", "Tab").get("accessibleItemSelected"), "Right did not select next pack")
    for _ in range(9): key("\uF703")
    last = wait_element("Shapes 12", "Tab")
    assert last.get("accessibleItemSelected"), "keyboard did not reach last pack"
    key("\uF702")
    wait_for(lambda: element("Shapes 11", "Tab").get("accessibleItemSelected"), "Left did not select previous pack")
    key("\uF703")
    wait_for(lambda: element("Shapes 12", "Tab").get("accessibleItemSelected"), "Right did not restore last pack")
    picker = bounds_by_id("StickerPicker::strip")
    assert last["absolutePosition"]["x"] >= picker["absolutePosition"]["x"] and last["absolutePosition"]["x"] + last["size"]["width"] <= picker["absolutePosition"]["x"] + picker["size"]["width"], "selected pack is clipped"
    check_pack_wheel()
    click("Happy shapes", "Tab")
    click("Send sticker 1")
    wait_for(lambda: any("Sticker" in item.get("accessibleLabel", "") and item.get("accessibleRole") in ("Text", "Image") for item in tree()), "sticker did not reach history")
    assert not any(item.get("accessibleLabel") == "Send sticker 1" for item in tree()), "picker stayed open after send"
    click("Stickers")
    wait_for(lambda: element("Sticker preview 1", "Image").get("accessibleDescription") == "Loaded", "cached picker did not reopen")
    key("\x1b")
    assert not any(item.get("accessibleLabel") == "Send sticker 1" for item in tree()), "Escape did not close sticker picker"
    click("Stickers")
    wait_element("Send sticker 1", "Button")
    click("Message draft", "TextInput")
    assert not any(item.get("accessibleLabel") == "Send sticker 1" for item in tree()), "outside click did not close sticker picker"
    show_chats()
    click("New chat")
    wait_element("Find friends for new chat", "TextInput")
    screenshot("new-chat")
    click("新しい友だち 🌸", "Checkbox")
    click("Chat")
    wait_for(lambda: any(item.get("accessibleLabel") == "新しい友だち 🌸" and item.get("accessibleRole") == "Button" for item in tree()), "friend conversation did not open")
    wait_element("Conversation avatar", "Image")
    screenshot("new-contact-header")
    draft_control = element("Message draft", "TextInput")
    call("click_element", elementHandle=draft_control["handle"])
    for char in "Hello from a new chat": key(char)
    wait_for(lambda: element("Message draft", "TextInput").get("accessibleValue") == "Hello from a new chat", "new friend composer did not accept typed text")
    click("Send")
    wait_for(lambda: any(item.get("accessibleValue") == "Hello from a new chat" for item in tree()), "first send to a friend without a chat failed")
    show_chats()
    wait_for(lambda: any(item.get("accessibleLabel", "").startswith("新しい友だち 🌸, ") for item in tree()), "first-send chat row missing")
    click("New chat")
    click("葵", "Checkbox")
    click("新しい友だち 🌸", "Checkbox")
    set_value("Group name", "Compose test group")
    screenshot("new-group")
    click("Create group")
    wait_for(lambda: any(item.get("accessibleLabel") == "Compose test group" for item in tree()), "created group did not open")
    # The portal is unavailable in headless runs; this guarded field injects local paths.
    import struct, zlib
    with tempfile.TemporaryDirectory(dir=UI_TEMP, prefix="line-attach-ui-") as directory:
        directory = Path(directory)
        def png_chunk(kind, data):
            return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
        photo = directory / "Holiday.png"
        scanlines = b"".join(b"\0" + bytes([58, 145, 191]) * 160 for _ in range(120))
        photo.write_bytes(b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", struct.pack(">IIBBBBB",160,120,8,2,0,0,0)) + png_chunk(b"IDAT", zlib.compress(scanlines)) + png_chunk(b"IEND", b""))
        file = directory / "Notes.txt"; file.write_text("Synthetic file")
        removed = directory / "Remove.txt"; removed.write_text("Remove this item")
        key("\uf70f")
        # Reopening retains the scroll position from earlier scenario checks.
        close = element("Close demo scenarios", "Button")
        call("dispatch_pointer_scroll", windowHandle=window,
             position={"x": close["absolutePosition"]["x"] + close["size"]["width"] / 2,
                       "y": close["absolutePosition"]["y"] + close["size"]["height"] + 60},
             deltaY=10000)
        paths = json.dumps([str(photo), str(file), str(removed)])
        click("Attachment paths", "TextInput")
        ctrl("a")
        key(paths)
        assert element("Attachment paths", "TextInput").get("accessibleValue") == paths, "attachment paths field is not typeable"
        click("Preview attachments")
        wait_element("Send 3", "Button")
        check_chain("attachments", ["Close attachments", "Remove Holiday.png", "Remove Notes.txt", "Remove Remove.txt", "Cancel", "Send 3"], ("Cancel", "Button"))
        screenshot("attachments")
        click("Remove Remove.txt")
        wait_element("Send 2", "Button")
        check_chain("attachments after removal", ["Close attachments", "Remove Holiday.png", "Remove Notes.txt", "Cancel", "Send 2"], ("Cancel", "Button"))
        assert not any(item.get("accessibleLabel", "").startswith("Uploading") for item in tree()), "selection uploaded before Send"
        click("Send 2")
        wait_for(lambda: any(item.get("accessibleLabel", "").startswith("Uploading ") and item.get("accessibleLabel") not in ("Uploading 0%", "Uploading 100%") for item in tree()), "upload progress did not appear")
        screenshot("uploading")
        wait_for(lambda: any(item.get("accessibleValue") == "Notes.txt" for item in tree()) and not any(item.get("accessibleLabel", "").startswith("Uploading") or item.get("accessibleLabel") == "Sending" for item in tree()), "attachment rows did not finish")


def check_scenario_keys():
    click("Message draft", "TextInput")
    for dismiss in ("escape", "close"):
        key("\uf70f")
        assert is_focused(scenario_element("Close demo scenarios"))
        if dismiss == "escape": key("\x1b")
        else: click("Close demo scenarios")
        assert is_focused(element("Message draft", "TextInput")), "scenarios did not restore the composer focus"
    def reverse_tab():
        key("\x10", "Press")
        key("\t")
        key("\x10", "Release")
    for closing_key in ("\x1b", " "):
        key("\uf70f")
        key("\t")
        assert is_focused(scenario_element("Long history")), "Tab from Close did not focus History"
        reverse_tab()
        assert is_focused(scenario_element("Close demo scenarios"))
        reverse_tab()
        assert is_focused(scenario_element("Offline")), "reverse Tab focused the wrong scenario"
        key(closing_key)
        assert not call("find_elements_by_id", windowHandle=window, elementsId="DemoScenarios::scenario-panel").get("elementHandles", []), "keyboard activation did not dismiss scenarios"
        key("\uf70f")
        assert is_focused(scenario_element("Close demo scenarios"))
        key("\t")
        assert is_focused(scenario_element("Long history")), "stale last-scenario focus trapped Tab after reopening"
        key("\x1b")
    key("\uf70f")
    call("click_element", elementHandle=scenario_action("Media failures")["handle"])
    reverse_tab()
    assert is_focused(scenario_element("Media cancelled")), "reverse Tab did not focus the final media scenario"
    key(" ")
    key("\uf70f")
    key("\t")
    assert is_focused(scenario_element("Long history")), "stale media focus trapped Tab after reopening"
    key("\x1b")


def check_scenarios():
    check_scenario_keys()
    reset_demo()
    demo_scenario("Long history")
    # The fixture page arrives on the worker. At wide size the original short
    # chat fits entirely, so scrolling must wait until the long page is committed.
    def history_scrollable():
        call("dispatch_pointer_scroll", windowHandle=window, position={"x": 600, "y": 280}, deltaY=800, deltaX=0)
        return any(item.get("accessibleLabel") == "Latest" for item in tree())
    wait_for(history_scrollable, "long history did not become scrollable")
    body = next(item for item in bodies() if "\n" in item.get("accessibleValue", "") and item["absolutePosition"]["y"] > 110)
    expected_text = body["accessibleValue"]
    expected_y = body["absolutePosition"]["y"]
    demo_scenario("Receive 3")
    call("click_element", elementHandle=body["handle"])
    ctrl("a")
    wait_for(lambda: element("Latest · 3 new", "Button"), "arrival burst did not finish")
    preserved = call("get_element_properties", elementHandle=body["handle"])
    assert preserved["absolutePosition"]["y"] == expected_y, "arrivals/media moved the reading anchor"
    wait_for(lambda: not any(item.get("accessibleLabel") == "Loading image…" for item in tree()), "history media did not finish loading")
    preserved = call("get_element_properties", elementHandle=body["handle"])
    assert preserved["absolutePosition"]["y"] == expected_y, "arrivals/media moved the reading anchor"
    ctrl("c")
    call("set_element_value", elementHandle=handle("Message draft", "TextInput"), value="")
    call("click_element", elementHandle=handle("Message draft", "TextInput"))
    ctrl("v")
    wait_for(lambda: wait_element("Message draft", "TextInput").get("accessibleValue") == expected_text, "clipboard lost selected text/emoji or included metadata")
    click("Latest · 3 new", "Button")
    assert not any("Latest" in item.get("accessibleLabel", "") for item in tree() if item.get("accessibleRole") == "Button"), "Latest did not resume following"
    burst_before = {item.get("accessibleValue") for item in bodies() if "到着 3" in item.get("accessibleValue", "")}
    demo_scenario("Receive 3")
    wait_for(lambda: any("到着 3" in item.get("accessibleValue", "") and item.get("accessibleValue") not in burst_before for item in bodies()), "at-latest burst did not arrive")
    assert any("到着 3" in item.get("accessibleValue", "") for item in bodies()), "at-latest burst did not remain visible"
    assert not any("Latest" in item.get("accessibleLabel", "") for item in tree() if item.get("accessibleRole") == "Button"), "at-latest burst disabled following"
    call("set_element_value", elementHandle=handle("Message draft", "TextInput"), value="Keep this newer draft")
    demo_scenario("Unanswered send")
    wait_for(lambda: any("Not sent" in item.get("accessibleLabel", "") for item in tree()), "unanswered send did not settle")
    assert not any(item.get("accessibleRole") == "Button" and item.get("accessibleLabel") in ("Retry connection", "Retry conversations", "Retry older") for item in tree()), "removed network Retry control returned"
    assert any("Not sent" in item.get("accessibleLabel", "") for item in tree()), "unanswered result was not attached to its outgoing item"
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == "Keep this newer draft", "late result replaced newer text"
    failed = next(item for item in bodies() if "Not sent" in item.get("accessibleLabel", ""))
    call("click_element", elementHandle=failed["handle"], button="Right")
    click("Copy to composer", "Text")
    click("Append", "Button")
    assert wait_element("Message draft", "TextInput").get("accessibleValue", "").startswith("Keep this newer draft\n"), "recovery overwrote the draft"


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

def check_sticker():
    assert not any(item.get("accessibleLabel") in ("Smaller text", "Larger text", "Text size") for item in tree()), "text-size controls leaked onto the main screen"
    wait_element("葵 avatar", "Image")
    click("Settings", "Button")
    wait_element("Message text size", "Combobox")
    key("\x1b")
    wait_element("Settings", "Button")
    demo_scenario("Sticker preview")
    wait_for(lambda: element("Sticker · [sticker]", "Text"), "sticker fallback did not appear")
    sticker_time = next(item["accessibleLabel"] for item in tree() if item.get("accessibleRole") == "Text" and item.get("accessibleLabel") in fixture_times("12:09"))
    before_sticker_time = element(sticker_time, "Text")["absolutePosition"]
    wait_element("Sticker", "Image")
    assert element(sticker_time, "Text")["absolutePosition"] == before_sticker_time, "sticker completion moved its timestamp"
    assert not any(item.get("accessibleLabel") == "Sticker · [sticker]" or item.get("accessibleValue") == "Sticker · [sticker]" for item in tree()), "loaded sticker retained its fallback label"

    click("Open photo", "Button")
    wait_element("Photo in viewer", "Image")
    key("\x1b")
    opener = element("Open photo", "Button")
    assert is_focused(opener), "sticker viewer did not return to its media control"
    focus_tab()
    assert_restored_focus("React")
    focus_tab(True)
    assert is_focused(opener), "reverse Tab from sticker React skipped its Open photo control"

    demo_scenario("Group preview")
    wait_element("Members", "Button")  # The group's details arrive after the fixture history.
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
    def day_label(date):
        year, month, day = date.year, date.month, date.day
        age = (datetime.date.today() - date).days
        return "Today" if age == 0 else "Yesterday" if age == 1 else date.strftime("%A") + f" {month}/{day}" if 2 <= age <= 6 else f"{year}/{month}/{day}"
    assert labels.count(day_label(datetime.date.today() - datetime.timedelta(days=1))) == 1 and labels.count(day_label(datetime.date.today())) == 1, "calendar dates were repeated or omitted"
    times = {item.get("accessibleLabel", "") for item in unique_group.values()
             if len(item.get("accessibleLabel", "")) == 16 and item["accessibleLabel"][4] == "/" and item["accessibleLabel"][10] == " "}
    yesterday_times = {(datetime.date.today() - datetime.timedelta(days=1)).strftime("%Y/%m/%d") + " " + clock for clock in ("09:19", "09:20", "09:27")}
    assert len(times) == 4 and yesterday_times <= times and times - yesterday_times <= fixture_times("09:18"), "five-minute/sender/day grouping showed the wrong timestamps"


def check_friends():
    call("set_element_value", elementHandle=handle("Message draft", "TextInput"), value=draft)
    click("Friends", "Button")
    wait_element("Open profile: 葵", "Button")  # Friends load on the engine worker.
    click("Settings")
    click("Message text size", "Combobox")
    click("Extra large")
    key("\x1b")
    ordered = sorted((item for item in tree() if item.get("accessibleRole") == "Button" and (item.get("accessibleLabel", "").startswith("Open profile:") or item.get("accessibleLabel") in ("Friends", "Groups"))), key=lambda item: item["absolutePosition"]["y"])
    rows = [item for item in ordered if item.get("accessibleLabel", "").startswith("Open profile:")]
    assert len(rows) > 2, "friend geometry fixture needs several profiles"
    height = rows[0]["size"]["height"]
    assert height > 0 and all(abs(row["size"]["height"] - height) < .01 for row in rows), "extra-large friend rows have unequal heights"
    for a, b in zip(ordered, ordered[1:]):
        if a in rows and b in rows:
            assert abs(b["absolutePosition"]["y"] - a["absolutePosition"]["y"] - height) < .01, f"extra-large friend row step differs from its height: {a} {b}"
    click("Settings")
    click("Message text size", "Combobox")
    click("Default")
    key("\x1b")
    call("set_element_value", elementHandle=handle("Search friends and groups", "TextInput"), value="葵")
    friend_rows = [item for item in tree() if item.get("accessibleLabel", "").startswith("Open profile:")]
    assert len(friend_rows) == 1 and friend_rows[0]["accessibleLabel"].startswith("Open profile: 葵"), "friend name search failed"
    call("click_element", elementHandle=handle("Search friends and groups", "TextInput"))
    key("\uf701")  # Category header
    key("\uf701")  # First friend
    key("\n")
    wait_element("Profile", "Text")
    assert any(item.get("accessibleLabel") == "Message draft" for item in tree()) == args.wide, "profile composer visibility disagrees with panel/sheet layout"
    click("Chats", "Button")
    assert not any(item.get("accessibleLabel") == "Close details" for item in tree()), "destination switch left profile open"
    click("Friends", "Button")
    click("Open profile: 葵", "Button")
    key("\x1b")
    wait_element("Search friends and groups", "TextInput")
    click("Chats", "Button")
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == draft, "friend browsing lost draft"
    click("Friends", "Button")
    assert wait_element("Search friends and groups", "TextInput").get("accessibleValue") == "葵", "section switch lost friend query"
    call("set_element_value", elementHandle=handle("Search friends and groups", "TextInput"), value="")
    click("Groups", "Button")
    assert not any(item.get("accessibleLabel", "").startswith("Open profile:") and "Weekend plans" in item.get("accessibleLabel", "") for item in tree()), "collapsed groups remained visible"
    click("Groups", "Button")
    click("Open profile: 新しい友だち 🌸", "Button")
    click("Chat", "Button")
    assert wait_element("Message draft", "TextInput").get("accessibleValue", "") == "", "new friend inherited another draft"
    wait_element("No messages yet", "Text")
    assert element("Chats", "Button").get("accessibleItemSelected") is True, "Open chat left the rail on Friends"
    wait_element("新しい友だち 🌸", "Button")  # local conversation header
    assert not any(item.get("accessibleLabel", "").startswith("新しい友だち 🌸, ") for item in tree()), "opening a friend created a chat row before its first send"
    for label in ("葵", "Weekend plans", "Notes to self"):
        wait_element(label, "Button")
    demo_scenario("Long chat list")
    click("Friends", "Button")
    click("Open profile: 新しい友だち 🌸", "Button")
    click("Chat", "Button")
    row = wait_for(lambda: next((item for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith("新しい友だち 🌸, ")), None), "far chat row was not revealed")
    viewport = element("Conversations", "List")
    assert row["absolutePosition"]["y"] >= viewport["absolutePosition"]["y"], "far row was not revealed"
    assert row["absolutePosition"]["y"] + row["size"]["height"] <= viewport["absolutePosition"]["y"] + viewport["size"]["height"] + 1, "far row remained clipped"
    click("Friends", "Button")
    call("set_element_value", elementHandle=handle("Search friends and groups", "TextInput"), value="葵")
    click("Open profile: 葵", "Button")
    click("Chat", "Button")
    click("葵", "Button")
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == draft, "opening a new friend lost prior draft"


def check_auth_recovery():
    before = wait_element("Message draft", "TextInput").get("accessibleValue")
    demo_scenario("Offline")
    wait_for(lambda: any(item.get("accessibleLabel") == "Connection needs attention" for item in tree()), "persistent network loss did not show Offline", timeout=3)
    wait_element("Connection needs attention", "Banner")
    assert not any(item.get("accessibleLabel") == "Sign in again" for item in tree()), "network failure offered sign-in"
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == before
    demo_scenario("Reconnect blip")
    wait_for(lambda: not any(item.get("accessibleLabel") == "Connection needs attention" for item in tree()), "network success did not clear Offline")

    before = wait_element("Message draft", "TextInput").get("accessibleValue")
    demo_scenario("Expired sign-in")
    wait_element("Connection needs attention", "Banner")
    call("click_element", elementHandle=handle("Message draft", "TextInput"))
    key("\uf709")  # F6 enters sign-in recovery.
    assert is_focused(element("Sign in again", "Button"))
    key("\n")
    assert any("Signing in here signs LINE out in Chrome." in item.get("accessibleLabel", "") for item in tree())
    assert is_focused(element("Start", "Button"))
    key("\x1b")
    assert is_focused(element("Sign in again", "Button"))
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == before, "sign-in dismissal changed the draft"
    key("\t")
    assert is_focused(element("Message history", "List"))
    click("Sign in again", "Button")
    click("Start", "Button")
    wait_for(lambda: any("LINE refused sign-in" in item.get("accessibleLabel", "") for item in tree()), "synthetic sign-in did not report its failure", timeout=3)
    key("\x1b")
    assert is_focused(element("Sign in again", "Button"))
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == before, "failed sign-in changed the draft"


def check_layers():
    demo_scenario("Protocol fixtures")
    # Public OA metadata assets use the same offline media paths as other photos.
    click("Open photo", "Button")
    wait_element("Photo in viewer", "Image")
    assert is_focused(element("Close photo", "Button")), "viewer did not focus Close photo on open"
    viewer_stops = ["Close photo", "Zoom out", "Zoom in", "Fit", "Original size", "Save photo"]
    viewer_stops += [label for label in ("Previous", "Next") if any(item.get("accessibleLabel") == label and item.get("accessibleEnabled") for item in tree())]
    check_chain("viewer", viewer_stops, ("Close photo", "Button"))
    key("\x1b")
    demo_scenario("Live reaction")
    wait_element("👍", "Text")
    wait_element("1", "Text")
    chip = wait_element("Emoji, 2 reactions", "Button")
    rows = tree(chip["handle"])
    assert not any(item.get("accessibleRole") == "Text" and item.get("accessibleLabel") == "Emoji" for item in rows), "paid chip displays its accessible name as text"
    wait_for(lambda: element("Emoji reaction", "Image").get("accessibleDescription") == "Loaded", "paid reaction asset did not load")
    mark = element("Emoji reaction", "Image")
    assert mark["size"]["width"] <= chip["size"]["width"] and mark["size"]["height"] <= chip["size"]["height"], "reaction asset overflows its chip"
    click("Emoji, 2 reactions")
    wait_element("Unknown member", "Text")
    wait_element("葵", "Text")
    wait_element("You", "Text")
    assert_reactor_popup_visible()
    screenshot("grouped-reactors")
    key("\x1b")
    assert is_focused(element("Emoji, 2 reactions", "Button"))
    focus_tab(True)
    assert_restored_focus("NICE, 1 reactions")
    focus_tab()
    assert_restored_focus("Emoji, 2 reactions")
    key("\n")
    assert_reactor_popup_visible()
    key("\x1b")

    def unsend_menu(body):
        message = next(item for item in tree() if body in item.get("accessibleLabel", "") and item.get("accessibleRole") in ("Text", "TextInput"))
        call("click_element", elementHandle=message["handle"], button="Right")
        click("Unsend", "Text")
        wait_element("Unsend message?", "Text")
        wait_element("It will be removed for everyone.", "Text")
        draft_before = wait_element("Message draft", "TextInput").get("accessibleValue")
        key("x")
        assert wait_element("Message draft", "TextInput").get("accessibleValue") == draft_before, "Unsend let typing reach the draft"

    unsend_menu("Unsend success fixture")
    key("\x1b")
    unsend_menu("Unsend success fixture")
    click("Cancel", "Button")
    unsend_menu("Unsend failure fixture")
    click("Unsend", "Button")
    wait_element("Couldn't unsend.", "Text")
    assert any("Unsend failure fixture" in item.get("accessibleLabel", "") for item in tree())
    unsend_menu("Unsend success fixture")
    click("Unsend", "Button")
    wait_for(lambda: any("Message unsent" in item.get("accessibleLabel", "") for item in tree()), "Unsend success did not replace the row", timeout=2)
    before = wait_element("Message draft", "TextInput").get("accessibleValue")
    click("Settings", "Button")
    close_settings = element("Close settings", "Button")
    if args.wide:
        dialog = bounds_by_id("Layers::settings-dialog")
        scale = properties.get("scaleFactor", 1)
        assert abs(dialog["size"]["width"] - min(640, properties["size"]["width"] / scale - 32)) <= 1, "Settings panel width changed"
        assert all(dialog["absolutePosition"][axis] > 0 and abs(dialog["absolutePosition"][axis] * 2 + dialog["size"][extent] - properties["size"][extent] / scale) <= 1 for axis, extent in (("x", "width"), ("y", "height"))), "wide Settings did not form a centered dialog"
    wait_element("Enter sends", "Checkbox")
    assert is_focused(call("get_element_properties", elementHandle=close_settings["handle"])), "Settings did not focus its close control"
    key("\t")
    assert is_focused(element("Settings section", "TabList")), "Settings navigation did not describe focus"
    for _ in range(12):
        key("\t")
    assert not is_focused(wait_element("Message draft", "TextInput")), "Settings let Tab escape to the draft"
    key("\x1b")
    assert is_focused(element("Navigation", "Button")), "Settings did not restore rail focus"
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == before
    click("Friends", "Button")
    call("set_element_value", elementHandle=handle("Search friends and groups", "TextInput"), value="葵")
    click("Open profile: 葵", "Button")
    assert any(item.get("accessibleLabel") == "Message draft" for item in tree()) == args.wide, "profile conversation visibility disagrees with layout"
    key("\x1b")
    assert is_focused(element("Friends and groups", "List")), "profile did not restore its list opener"
    click("Chats", "Button")
    demo_scenario("Photo preview")
    click("Open photo", "Button")
    click("Next", "Button")
    wait_for(lambda: any("800 × 1200" in item.get("accessibleLabel", "") for item in tree()), "Next left the loaded photo scope")
    click("Previous", "Button")
    wait_for(lambda: any("1200 × 800" in item.get("accessibleLabel", "") for item in tree()), "Previous did not load the first photo")
    click("Original size")
    wait_for(lambda: abs(element("Photo in viewer", "Image")["size"]["width"] - 1200) < .01, "original-size view did not settle")
    assert element("Photo in viewer", "Image")["size"]["width"] >= 1199, "original-size view only enlarged the thumbnail"
    click("Fit")
    key("1")
    wait_for(lambda: abs(element("Photo in viewer", "Image")["size"]["width"] - 1200) < .01, "original-size shortcut did not settle")
    image = element("Photo in viewer", "Image")
    assert image["size"]["width"] >= 1199, "1 did not show original size"
    pointer = {"x": 400, "y": 300}
    call("dispatch_pointer_scroll", windowHandle=window, position=pointer, deltaY=80, modifiers={"control": True})
    wait_for(lambda: abs(element("Photo in viewer", "Image")["size"]["width"] - image["size"]["width"] * 1.2) < .01, "wheel zoom did not settle")
    zoomed = element("Photo in viewer", "Image")
    assert zoomed["size"]["width"] > image["size"]["width"], "Ctrl+wheel did not zoom"
    fraction = (pointer["x"] - image["absolutePosition"]["x"]) / image["size"]["width"]
    assert abs(zoomed["absolutePosition"]["x"] + fraction * zoomed["size"]["width"] - pointer["x"]) < 3, "wheel zoom moved the point under the pointer"
    center = {"x": zoomed["absolutePosition"]["x"] + zoomed["size"]["width"] / 2 + 80, "y": 350}
    call("drag_element", elementHandle=zoomed["handle"], target=center)
    assert abs(element("Photo in viewer", "Image")["absolutePosition"].get("x", 0) - zoomed["absolutePosition"]["x"]) > 20, "drag did not pan"
    click("Fit")
    wait_for(lambda: element("Photo in viewer", "Image")["size"]["width"] <= properties["size"]["width"] + 1 and element("Photo in viewer", "Image")["size"]["height"] <= properties["size"]["height"] + 1, "fit did not settle")
    fitted = element("Photo in viewer", "Image")["size"]
    assert fitted["width"] <= properties["size"]["width"] + 1 and fitted["height"] <= properties["size"]["height"] + 1, "Fit did not restore a bounded view"
    key("1")
    key("0")
    wait_for(lambda: element("Photo in viewer", "Image")["size"]["width"] <= properties["size"]["width"] + 1 and element("Photo in viewer", "Image")["size"]["height"] <= properties["size"]["height"] + 1, "fit did not settle")
    fitted = element("Photo in viewer", "Image")["size"]
    assert fitted["width"] <= properties["size"]["width"] + 1 and fitted["height"] <= properties["size"]["height"] + 1, "0 did not fit the photo"
    click("Save photo", "Button")
    saved = wait_for(lambda: [item.get("accessibleLabel", "") for item in tree() if item.get("accessibleLabel", "").startswith("Saved to ")], "Save did not report a completed export")
    exported = Path(saved[0].removeprefix("Saved to "))
    assert exported.is_relative_to(demo_directory / "exports"), "preview export escaped its isolated folder"
    assert exported.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
    exported.unlink()
    key("\x1b")
    assert any(item.get("accessibleLabel") == "Open photo" and is_focused(item) for item in tree()), "viewer did not restore the photo opener"
    call("hover_element", elementHandle=element("Open photo", "Button")["handle"])
    focus_tab()
    assert_restored_focus("React")
    focus_tab(True)
    assert is_focused(element("Open photo", "Button")), "reverse Tab from photo React skipped that message's Open photo control"
    assert focused_control()["visible"], "reverse Tab from photo React focused hidden content"
    assert focused_control().get("role") != "TextInput", "reverse Tab from photo React focused its invisible body"
    photos = [item for item in tree() if item.get("accessibleLabel") == "Open photo" and item.get("accessibleRole") == "Button"]
    call("click_element", elementHandle=photos[0]["handle"], button="Right")
    click("Open photo", "Text")
    key("\x1b")
    assert is_focused(element("Open photo", "Button")), "context-menu viewer did not return focus to its photo"
    click("Open photo")
    click("Next")
    click("Close photo")
    photos = [item for item in tree() if item.get("accessibleLabel") == "Open photo" and item.get("accessibleRole") == "Button"]
    assert is_focused(photos[-1]), "closing after Next did not focus the displayed photo"
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == before, "layers changed the draft"
    # Cache the members while connected, then exercise the same layer signed out.
    click("Weekend plans")
    click("Members")
    wait_for(lambda: element("Open member: 葵", "Button"), "connected group members did not load")
    key("\x1b")
    demo_scenario("Expired sign-in")
    click("Friends", "Button")
    click("Open profile: 葵", "Button")
    wait_for(lambda: element("Close details", "Button"), "profile did not open")
    assert any(item.get("accessibleLabel") == "Connection needs attention" for item in tree()) == args.wide, "profile recovery visibility disagrees with panel/sheet layout"
    key("\x1b")
    click("Chats", "Button")
    click("Weekend plans", "Button")
    wait_for(lambda: element("Connection needs attention", "Banner"), "recovery strip did not appear")
    click("Members", "Button")
    wait_element("Members (25)", "Text")
    for name in ("葵", "蓮"):
        set_value("Search members", name)
        wait_for(lambda: element("Open member: " + name, "Button"), f"member {name} did not appear")
    key("\x1b")


def check_virtual_wheel():
    def latest_row():
        return any(item.get("accessibleRole") == "TextInput" and "History item 0999" in item.get("accessibleValue", "") for item in tree())
    wait_for(latest_row, "paging history did not reach its latest page")
    # The first render can precede the full page. Reopen its cached window to select virtual layout.
    click("Weekend plans")
    click("葵")
    wait_for(lambda: call("find_elements_by_id", windowHandle=window, elementsId="Conversation::history-view").get("elementHandles"), "wheel fixture must use virtual history")
    wait_for(latest_row, "virtual history did not reach its latest page")
    def latest():
        return any(item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith("Latest") for item in tree())
    def wheel(delta):
        call("dispatch_pointer_scroll", windowHandle=window, position={"x": 650, "y": 280}, deltaY=delta, deltaX=0)
    wheel(120)
    wait_for(latest, "one wheel notch in virtual history did not show Latest")
    wheel(-1000)
    wait_for(lambda: not latest(), "wheel to the bottom did not resume following")
    wheel(100000)
    wait_for(lambda: any("History item " in item.get("accessibleValue", "") and int(item["accessibleValue"].split("History item ")[1][:4]) < 920 for item in tree()), "wheel at the top did not load older history")



def check_reading():
    def reading_bodies():
        return [item for item in tree() if item.get("accessibleRole") in ("TextInput", "Text") and item.get("accessibleLabel", "").startswith(("Incoming from ", "Outgoing, "))]

    demo_scenario("Reading fixtures")
    wait_for(lambda: any(item.get("accessibleLabel") == "Beginning of conversation" for item in tree()), "automatic reading-window refresh did not finish", timeout=5)
    labels = [item.get("accessibleLabel", "") for item in tree()]
    # MCP exposes clipped rows only while visible; inspect both fixture halves.
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 650, "y": 280}, deltaY=-1800, deltaX=0)
    labels.extend(item.get("accessibleLabel", "") for item in tree())
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 650, "y": 280}, deltaY=1800, deltaX=0)
    for expected in ("Photo", "Sticker", "Video", "Voice message", "File", "Location", "Contact", "joined the group", "Message unsent", "Couldn't decrypt this message", "This message type isn't supported yet"):
        assert any(expected in label for label in labels), f"missing placeholder: {expected}"
    first = next(item for item in reading_bodies() if "https://example.org/reading" in item.get("accessibleValue", ""))
    url = "https://example.org/reading"
    link = element("Open link " + url, "Button")
    assert link.get("accessibleDescription") == url, "full link cannot be inspected"
    call("invoke_accessibility_action", elementHandle=link["handle"], action="Default_")
    assert element("Recorded link activation", "Text").get("accessibleValue") == url, "demo did not record link without external browser"
    before_copy = wait_element("Message draft", "TextInput").get("accessibleValue", "")
    other_link = element("Open link https://www.example.jp", "Button")
    target = other_link["absolutePosition"]
    call("drag_element", elementHandle=other_link["handle"], target={"x": target["x"] + 2, "y": target["y"] + 2})
    assert element("Recorded link activation", "Text").get("accessibleValue") == url, "drag activated a link"
    ctrl("c")
    call("set_element_value", elementHandle=handle("Message draft", "TextInput"), value="")
    call("click_element", elementHandle=handle("Message draft", "TextInput"))
    ctrl("v")
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == first.get("accessibleValue"), "dragging URL action did not select original message text"
    call("set_element_value", elementHandle=handle("Message draft", "TextInput"), value=before_copy)
    click("Show message", "Button")
    first = next(item for item in reading_bodies() if "https://example.org/reading" in item.get("accessibleValue", ""))
    link = element("Open link " + url, "Button")
    call("click_element", elementHandle=link["handle"], button="Right")
    copy_link = next(item for item in tree() if item.get("accessibleLabel") == "Copy link")
    call("click_element", elementHandle=copy_link["handle"])
    call("set_element_value", elementHandle=handle("Message draft", "TextInput"), value="")
    call("click_element", elementHandle=handle("Message draft", "TextInput"))
    ctrl("v")
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == url, "Copy link did not copy normalized address"
    call("set_element_value", elementHandle=handle("Message draft", "TextInput"), value=before_copy)

    call("click_element", elementHandle=handle("Message draft", "TextInput"))
    # Reverse traversal reaches the single history surface, regardless of rows.
    # Tab follows the composer's visual order: Attach and Stickers precede the draft.
    # The history pills sit between the history and the composer.
    composer = [label for label in ("Attach", "Stickers") if element(label, "Button").get("accessibleEnabled")]
    # Reverse Tab visits Return before Latest.
    pills = [item["accessibleLabel"] for prefix in ("Return to message", "Latest") for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith(prefix)]
    for label in composer + pills + ["Message history"]:
        key("\x10", "Press")
        key("\t")
        key("\x10", "Release")
        stop = element(label, "List" if label == "Message history" else "Button")
        assert is_focused(stop), f"reverse Tab from the draft missed {label}: at {focus_label()}"
    history = stop
    key("\uf72b")  # End
    bottom = call("get_element_properties", elementHandle=first["handle"])["absolutePosition"]["y"]
    key("\uf729")  # Home
    top = call("get_element_properties", elementHandle=first["handle"])["absolutePosition"]["y"]
    assert top > bottom, "Home did not return to history beginning"
    key("\uf701")  # Down
    down = call("get_element_properties", elementHandle=first["handle"])["absolutePosition"]["y"]
    assert down < top, "Down did not scroll history"
    key("\uf700")  # Up
    up = call("get_element_properties", elementHandle=first["handle"])["absolutePosition"]["y"]
    assert up > down, "Up did not scroll history"
    key("\uf72b")  # End
    key("\uf72c")  # PageUp
    wait_element("Latest", "Button")
    key("\uf72d")  # PageDown
    key("\uf72b")  # End
    last = next(item for item in reading_bodies() if "This message type isn't supported yet" in item.get("accessibleValue", ""))
    call("click_element", elementHandle=last["handle"])
    assert not any(item.get("accessibleLabel") == "Latest" and item.get("accessibleRole") == "Button" for item in tree()), "click focus disabled following"
    key("\uf700")  # Up from a click-focused body scrolls on the first press.
    wait_element("Latest", "Button")
    call("click_element", elementHandle=last["handle"])
    # Selection release can be emitted synchronously while a chat model is
    # replaced; it must queue deferred work rather than borrow View reentrantly.
    ctrl("a")
    click("Weekend plans", "Button")
    click("葵", "Button")
    assert element("Message draft", "TextInput").get("accessibleValue") == before_copy, "selected-body chat switch lost draft or failed"
    demo_scenario("Reading fixtures")


def check_rich():
    demo_scenario("Rich content")
    wait_element("[笑]", "Image")
    rich_body = next(item for item in tree() if item.get("accessibleRole") == "Text" and "日😀[笑]本🌸" in item.get("accessibleLabel", ""))
    call("click_element", elementHandle=rich_body["handle"])
    for reverse, label in ((False, "React"), (False, "Reply"), (True, "React"), (True, rich_body["accessibleLabel"])):
        focus_tab(reverse)
        stop = rich_body if label == rich_body["accessibleLabel"] else element(label, "Button")
        assert is_focused(stop), f"rich message Tab skipped {label}"
    click("Message draft", "TextInput")
    title = "Rich content with a very long conversation name 日本語"
    chat = wait_element(title, "Button")
    rows = tree(chat["handle"])
    name = next(r for r in rows if any(t.get("id", "").endswith("::name") for t in r.get("typeNamesAndIds", [])))
    assert name.get("accessibleLabel") == title and name.get("accessibleDescription") == "Truncated", "long chat name did not elide with its full accessible name"
    date = next(r for r in rows if any(t.get("id", "").endswith("::time") for t in r.get("typeNamesAndIds", [])))
    assert date.get("accessibleLabel") == chat["accessibleLabel"].split(", ")[1], "chat date text is missing"
    # Accessibility exposes full text even if clipped; check containment and separation too.
    assert name["absolutePosition"]["x"] + name["size"]["width"] <= date["absolutePosition"]["x"], "name covers chat date"
    assert date["absolutePosition"]["x"] + date["size"]["width"] <= chat["absolutePosition"]["x"] + chat["size"]["width"], "chat date is clipped"
    screenshot("long-chat-name")
    metadata_rows = tree()
    labels = [item.get("accessibleLabel", "") for item in metadata_rows]
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 800, "y": 400}, deltaY=-800, deltaX=0)
    metadata_rows += tree()
    labels += [item.get("accessibleLabel", "") for item in metadata_rows]
    assert any(item.get("accessibleValue") == "葵" and any(base.get("id", "").endswith("::card-title") for base in item.get("typeNamesAndIds", [])) for item in metadata_rows), "contact card lost its name"
    for expected in ("2.0 KB", "2:05", "0:12", "葵", "葵 joined the group", "Message unsent"):
        assert any(expected in label for label in labels), f"missing rich metadata: {expected}"
    call("dispatch_pointer_scroll", windowHandle=window, position={"x": 800, "y": 400}, deltaY=1600, deltaX=0)
    wait_element("Jump to replied message")
    def emoji_body():
        return next(item for item in tree() if item.get("accessibleRole") == "Text" and "日😀[笑]本🌸" in item.get("accessibleLabel", ""))
    has_latest = any(item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith("Latest") for item in tree())
    assert has_latest == (properties["size"]["height"] / properties.get("scaleFactor", 1) < 900), "rich fixture scrolling disagrees with window size"
    if properties["size"]["height"] / properties.get("scaleFactor", 1) < 900:
        for _ in range(2):
            latest = next(item for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith("Latest"))
            call("click_element", elementHandle=latest["handle"])
            assert not any(item.get("accessibleLabel") == "Return to message" for item in tree()), "Latest retained the return anchor"
            latest_y = emoji_body()["absolutePosition"]["y"]
            click("Jump to replied message", "Button")
            wait_for(lambda: abs(emoji_body()["absolutePosition"]["y"] - latest_y) > 1, "reply jump after Latest did not scroll")
            click("Return to message", "Button")
            wait_for(lambda: abs(emoji_body()["absolutePosition"]["y"] - latest_y) <= 1, "Return after Latest lost its origin")
        call("dispatch_pointer_scroll", windowHandle=window, position={"x": 800, "y": 400}, deltaY=1600, deltaX=0)
        reply = element("Jump to replied message", "Button")
        origin_y = emoji_body()["absolutePosition"]["y"]
        call("click_element", elementHandle=reply["handle"])
        demo_scenario("Offline")
        wait_for(lambda: any(item.get("accessibleLabel") == "Connection needs attention" for item in tree()), "persistent loss did not show Offline", timeout=3)
        wait_element("Return to message", "Button")
        latest = next(item for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith("Latest"))
        return_pill = element("Return to message", "Button")
        assert abs(latest["absolutePosition"]["y"] - return_pill["absolutePosition"]["y"]) <= 1, "history pills stacked vertically"
        assert latest["absolutePosition"]["x"] + latest["size"]["width"] <= return_pill["absolutePosition"]["x"], "history pills overlap"
        strip = element("Connection needs attention", "Banner")
        history = element("Message history", "List")
        editor = element("Message draft", "TextInput")
        assert strip["absolutePosition"]["y"] == history["absolutePosition"]["y"], "strip left the history top"
        for pill in (latest, return_pill):
            assert strip["absolutePosition"]["y"] + strip["size"]["height"] <= pill["absolutePosition"]["y"], "strip covered a history pill"
            assert pill["absolutePosition"]["y"] + pill["size"]["height"] <= editor["absolutePosition"]["y"], "history pill covered the composer"
        # Tab follows the composer's visual order
        composer_start = ("Stickers", "Button") if element("Stickers", "Button").get("accessibleEnabled") else ("Attach", "Button")
        call("click_element", elementHandle=handle("Message draft", "TextInput"))
        # Attach is the first enabled stop before the draft in reverse order.
        composer_reverse = [(label, "Button") for label in ("Attach", "Stickers") if element(label, "Button").get("accessibleEnabled")]
        for label, role in [*composer_reverse, ("Return to message", "Button"), (latest["accessibleLabel"], "Button"), ("Message history", "List")]:
            key("\x10", "Press")
            key("\t")
            key("\x10", "Release")
            assert is_focused(element(label, role)), f"reverse Tab skipped {label}"
        assert is_focused(element("Message history", "List")), "reverse Tab skipped a history pill"
        for label, role in [(latest["accessibleLabel"], "Button"), ("Return to message", "Button"), composer_start]:
            key("\t")
            assert is_focused(element(label, role)), f"forward Tab skipped {label}"
        assert is_focused(wait_element(*composer_start)), "history/Latest/Return/composer Tab order changed"
        click("Return to message", "Button")
        wait_for(lambda: abs(emoji_body()["absolutePosition"]["y"] - origin_y) <= 1, "reply return did not restore its origin")
        restored_y = emoji_body()["absolutePosition"]["y"]
        assert abs(restored_y - origin_y) <= 1, f"Offline appearing moved reply return: {origin_y} -> {restored_y}"
        # Capture with the strip present, then let the local reconnect fixture clear it.
        origin_y = emoji_body()["absolutePosition"]["y"]
        reply = element("Jump to replied message", "Button")
        quote_y = reply["absolutePosition"]["y"]
        call("click_element", elementHandle=reply["handle"])
        demo_scenario("Reconnect blip")
        wait_for(lambda: not any(item.get("accessibleRole") == "Banner" and item.get("accessibleLabel") in ("Connection needs attention", "Connection restored") for item in tree()), "synthetic reconnect did not clear the strip", timeout=7)
        click("Return to message", "Button")
        wait_for(lambda: abs(emoji_body()["absolutePosition"]["y"] - origin_y) <= 1, "reply return did not restore its origin")
        restored_y = emoji_body()["absolutePosition"]["y"]
        assert abs(restored_y - origin_y) <= 1, f"Offline disappearing moved reply return: {origin_y} -> {restored_y}"
        reply = element("Jump to replied message", "Button")
        assert abs(reply["absolutePosition"]["y"] - quote_y) <= 1, "Return moved the reply quote"
        history = element("Message history", "List")
        viewports = call("find_elements_by_id", windowHandle=window, elementsId="Pane::viewport")["elementHandles"]
        viewport = next(item for handle in viewports
                        if -1 <= (item := call("get_element_properties", elementHandle=handle))["absolutePosition"].get("x", 0) - history["absolutePosition"].get("x", 0) <= 16
                        and abs(item["size"]["width"] - (history["size"]["width"] - 12)) <= 1)
        assert abs(viewport["absolutePosition"]["y"] - history["absolutePosition"]["y"]) <= 1, "Return left a blank residual band above the anchor"
        # Retain the unchanged-inset regression as well.
        origin_y = restored_y
        click("Jump to replied message", "Button")
        wait_element("Return to message", "Button")  # The restored quote must accept another jump.
        click("Return to message", "Button")
        wait_for(lambda: abs(emoji_body()["absolutePosition"]["y"] - origin_y) <= 1, "reply return lost the reading anchor with unchanged inset")
    else:
        # At 1280×900 this eight-row fixture fits; no Latest pill or scroll is needed.
        origin_y=emoji_body()["absolutePosition"]["y"]
        click("Jump to replied message","Button")
        click("Return to message","Button")
        assert abs(emoji_body()["absolutePosition"]["y"]-origin_y)<=1, "fit-history reply return moved content"
    call("click_element", elementHandle=emoji_body()["handle"], button="Right")
    select_text = next(item for item in tree() if item.get("accessibleLabel") == "Select text")
    call("click_element", elementHandle=select_text["handle"])
    body = next(item for item in tree() if item.get("accessibleRole") == "TextInput" and "日😀[笑]本🌸" in item.get("accessibleValue", ""))
    original = body["accessibleValue"]
    call("click_element", elementHandle=body["handle"])
    ctrl("a")
    ctrl("c")
    call("set_element_value", elementHandle=handle("Message draft", "TextInput"), value="")
    call("click_element", elementHandle=handle("Message draft", "TextInput"))
    ctrl("v")
    wait_for(lambda: wait_element("Message draft", "TextInput").get("accessibleValue") == original, "rich copy lost emoji fallback or newlines")
    click("Show emoji", "Button")
    wait_element("[笑]", "Image")
    exports = demo_directory / "exports"
    existing = set(exports.glob("*"))
    click("Save", "Button")
    wait_for(lambda: element("Saved", "Text"), "file Save did not complete")
    saved = set(exports.glob("*")) - existing
    assert any(path.read_bytes() == b"Synthetic attachment\n" for path in saved if path.is_file()), "demo export bytes differ"
    for path in saved:
        path.unlink()


def check_media():
    cases = [
        ("Media timeout", "Image took too long to load", True),
        ("Media 404", "Image is no longer available", False),
    ]
    for scenario, reason, retriable in cases:
        demo_scenario("Media failures")
        call("click_element", elementHandle=scenario_action(scenario)["handle"])
        wait_element(reason, "Text")
        retries = [item for item in tree() if item.get("accessibleRole") == "Button" and item.get("accessibleLabel") == "Retry image"]
        assert bool(retries) == retriable, (scenario, retries)
        assert not any("Refresh" in item.get("accessibleLabel", "") for item in tree())
        if retriable:
            call("click_element", elementHandle=retries[0]["handle"])
            wait_for(lambda: not any(item.get("accessibleLabel") == reason for item in tree()), "Retry kept the cached failure")


def check_unread():
    demo_scenario("Long history")
    wait_for(lambda: any("39: compact message" in item.get("accessibleValue", item.get("accessibleLabel", "")) for item in tree()), "long history page was not applied")
    def scroll_unread_history():
        call("dispatch_pointer_scroll",windowHandle=window,position={"x":600,"y":280},deltaY=700,deltaX=0)
        return any(item.get("accessibleLabel") == "Latest" for item in tree())
    wait_for(scroll_unread_history, "long history did not scroll away from Latest")
    candidates=[item for item in tree() if item.get("accessibleRole")=="TextInput" and any(base.get("id","").endswith("::body") for base in item.get("typeNamesAndIds",[]))]
    assert candidates, "unread reading anchor was missing"
    anchor=candidates[0]
    demo_scenario("Unread scenario")
    def unread_badge(label, count):
        return any(item.get("accessibleRole") == "Button" and item.get("accessibleLabel", "").startswith(label + ", ") and f", {count} unread" in item["accessibleLabel"] for item in tree())
    wait_for(lambda: unread_badge("葵", 3), "open-chat arrivals lost unread before exposure")
    wait_for(lambda: unread_badge("Weekend plans", 2), "other-chat unread badge did not update")
    after=next(item for item in tree() if item.get("accessibleRole")==anchor.get("accessibleRole") and item.get("accessibleValue")==anchor.get("accessibleValue"))
    assert abs(after["absolutePosition"]["y"]-anchor["absolutePosition"]["y"])<=1, f"unread arrivals moved content: {anchor['absolutePosition']} -> {after['absolutePosition']}"
    wait_element("Latest · 3 new","Button")
    # The divider can be outside the bounded rendered range while scrolled up.
    click("Latest · 3 new","Button")
    # A background automation window must stay unread. An already-active owner
    # may expose and read all three; focus is never requested by this test.
    current=element("葵","Button")
    assert not any(f", {n} unread" in current["accessibleLabel"] for n in [1,2]),"complete exposure left a partial boundary"
    wait_element("Unread messages","Text")
    assert ", 2 unread" in element("Weekend plans","Button")["accessibleLabel"],"reading one chat cleared another chat"


def check_action_geometry():
    demo_scenario("Message actions")
    for chat in ("Message actions", "Group actions"):
        if chat == "Group actions":
            click("Group actions")
            wait_element("Members, 25")
            wait_for(lambda: any(r.get("accessibleLabel", "").startswith("Profile: ") for r in tree()), "group sender avatars missing")
        wait_for(lambda: len([b for b in bodies() if b.get("accessibleValue", "").startswith(("Incoming ", "Outgoing "))]) == 6, "action fixture did not load")
        bubbles = call("find_elements_by_id", windowHandle=window, elementsId="Message::message-bubble")["elementHandles"]
        bubble_bounds = [call("get_element_properties", elementHandle=h) for h in bubbles]
        contents = call("find_elements_by_id", windowHandle=window, elementsId="Message::message-content")["elementHandles"]
        content_bounds = [call("get_element_properties", elementHandle=h) for h in contents]
        times = call("find_elements_by_id", windowHandle=window, elementsId="Message::time-slot")["elementHandles"]
        time_bounds = [call("get_element_properties", elementHandle=h) for h in times]
        for prefix in ("Incoming first", "Incoming middle", "Incoming last", "Outgoing first", "Outgoing middle", "Outgoing last"):
            body = next(b for b in bodies() if b.get("accessibleValue", "").startswith(prefix))
            call("hover_element", elementHandle=body["handle"])
            rows = tree()
            react = min((r for r in rows if r.get("accessibleRole") == "Button" and r.get("accessibleLabel") == "React"), key=lambda r: abs(r["absolutePosition"]["y"] - body["absolutePosition"]["y"]))
            reply = min((r for r in rows if r.get("accessibleRole") == "Button" and r.get("accessibleLabel") == "Reply"), key=lambda r: abs(r["absolutePosition"]["y"] - react["absolutePosition"]["y"]))
            rx, ry = react["absolutePosition"]["x"], react["absolutePosition"]["y"]
            bubble = min(bubble_bounds, key=lambda b: abs(b["absolutePosition"]["y"] - body["absolutePosition"]["y"]))
            outgoing = prefix.startswith("Outgoing")
            content = min(content_bounds, key=lambda r: abs(r["absolutePosition"]["y"] - bubble["absolutePosition"]["y"]))
            if outgoing:
                assert abs(bubble["absolutePosition"]["x"] + bubble["size"]["width"] - content["absolutePosition"]["x"] - content["size"]["width"]) <= 1, f"{prefix}: bubble left the row's right edge"
            else:
                gutters = call("find_elements_by_id", windowHandle=window, elementsId="Message::sender-gutter").get("elementHandles", [])
                gutters = [call("get_element_properties", elementHandle=h) for h in gutters]
                gutter = next((g for g in gutters if abs(g["absolutePosition"]["y"] - content["absolutePosition"]["y"]) <= 1), None)
                left = content["absolutePosition"]["x"] + (gutter["size"]["width"] + 5 if gutter else 0)
                assert abs(bubble["absolutePosition"]["x"] - left) <= 1, f"{prefix}: bubble x={bubble['absolutePosition']['x']} left the row content edge {left}; content={content}"
            bottom = bubble["absolutePosition"]["y"] + bubble["size"]["height"]
            assert abs(ry + react["size"]["height"] - bottom) <= 1, f"{prefix}: actions left the bubble's bottom row"
            time_slot = min(time_bounds, key=lambda t: abs(t["absolutePosition"]["y"] + t["size"]["height"] - bottom))
            shows_time = time_slot["size"].get("width", 0) > 0
            assert shows_time == prefix.endswith("last"), f"{prefix}: time must appear only at the end of the run"
            anchor = time_slot if shows_time else bubble
            ax, aw = anchor["absolutePosition"]["x"], anchor["size"]["width"]
            edge = reply["absolutePosition"]["x"] + reply["size"]["width"] if outgoing else rx
            distance = ax - edge if outgoing else edge - ax - aw
            assert 0 <= distance <= 5, f"{prefix}: actions are not beside {'time' if shows_time else 'bubble'}: {distance} px"
            assert abs(reply["absolutePosition"]["x"] - rx - react["size"]["width"] - 2) <= 1, f"{prefix}: React and Reply overlap or gap is not 2 px"
            assert abs(reply["absolutePosition"]["y"] - ry) <= 1, f"{prefix}: actions are not side by side"
            for action in (react, reply):
                x, y = action["absolutePosition"]["x"], action["absolutePosition"]["y"]
                right, bottom = x + action["size"]["width"], y + action["size"]["height"]
                for other in bubble_bounds:
                    if other == bubble:
                        continue
                    ox, oy = other["absolutePosition"]["x"], other["absolutePosition"]["y"]
                    assert right <= ox or x >= ox + other["size"]["width"] or bottom <= oy or y >= oy + other["size"]["height"], f"{prefix}: {action['accessibleLabel']} overlaps another message bubble"
            screenshot(chat.lower().replace(" ", "-") + "-" + prefix.lower().replace(" ", "-"))


SCENES = {'virtual-wheel': 'paging'}

GROUPS = {
    'virtual-wheel': check_virtual_wheel,
    'action-geometry': check_action_geometry,
    'smoke': check_smoke,
    'emoji': check_emoji,
    'focus': check_focus,
    'compose': check_compose,
    'scenarios': check_scenarios,
    'sticker': check_sticker,
    'friends': check_friends,
    'layers': check_layers,
    'reading': check_reading,
    'rich': check_rich,
    'media': check_media,
    'unread': check_unread,
    'detail-stack': check_detail_stack,
    'auth-recovery': check_auth_recovery,
    'signed-out': check_signed_out,
    'missing-module': check_missing_module,
}
WIDE_GROUPS = ('smoke', 'layers', 'focus', 'friends', 'scenarios')


def connect(process):
    global window, root, properties, connected
    connected = False
    def initialize():
        assert process.poll() is None, 'headless demo exited before MCP initialization'
        return rpc('initialize', {'protocolVersion': '2025-03-26', 'capabilities': {},
                                 'clientInfo': {'name': 'line-ui-check', 'version': '2'}}, timeout=1)
    def ready():
        try:
            return initialize()
        except (urllib.error.URLError, TimeoutError, ConnectionError):
            assert process.poll() is None, 'headless demo crashed during MCP initialization'
            return None
    wait_for(ready, 'MCP initialize did not answer within 30 seconds', 30)
    windows = wait_for(lambda: call('list_windows')['windowHandles'], 'demo window did not appear')
    window = windows[0]
    properties = call('get_window_properties', windowHandle=window)
    root = properties['rootElementHandle']
    if args.wide:
        scale = properties.get('scaleFactor', 1)
        assert (properties['size']['width'] / scale, properties['size']['height'] / scale) == (1280, 900), '--wide requires a 1280×900 window'
    markers = call('find_elements_by_id', windowHandle=window, elementsId='DemoExtension::demo-reset-marker').get('elementHandles', [])
    if len(markers) != 1:
        raise WrongInstanceError('connected instance is not an automation demo')
    marker = call('get_element_properties', elementHandle=markers[0])
    if marker.get('accessibleLabel') != process.demo_token:
        raise WrongInstanceError('connected instance is not a demo')
    if marker.get('accessibleValue') != process.demo_token + ':0':
        raise WrongInstanceError('MCP answered from the wrong process')
    connected = True


def reset_demo():
    global draft, before_shortcut, window, root, properties
    for _ in range(4):
        key('\x1b')
    key("\uf70f")
    generation = bounds_by_id('DemoExtension::demo-reset-marker')['accessibleValue']
    token, number = generation.rsplit(':', 1)
    expected = f'{token}:{int(number) + 1}'
    click('Reset demo')
    def reset_window():
        global window, root, properties
        for candidate in call('list_windows')['windowHandles']:
            try:
                markers = call('find_elements_by_id', windowHandle=candidate, elementsId='DemoExtension::demo-reset-marker')['elementHandles']
            except LookupError:
                continue
            if len(markers) != 1:
                continue
            marker = call('get_element_properties', elementHandle=markers[0])
            if marker.get('accessibleLabel') == token and marker.get('accessibleValue') == expected:
                window = candidate
                properties = call('get_window_properties', windowHandle=window)
                root = properties['rootElementHandle']
                return True
        return False
    wait_for(reset_window, 'demo reset did not complete')
    wait_element('葵')
    smoke_chat('葵')
    draft = '日本語の下書き\nDraft with emoji 🐈'
    before_shortcut = draft + '\n'
    set_value('Message draft', draft)


def main():
    global args, current_group, demo_directory, demo_pid
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, help='Reproduce the randomized --all group order')
    parser.add_argument('--all', action='store_true', help='Run the normal, wide, and narrow behavioural matrix')
    parser.add_argument('--group', choices=GROUPS, action='append', help='Run a group (repeatable; defaults to smoke)')
    parser.add_argument('--dark', action='store_true', help='Use the dark demo palette')
    parser.add_argument('--wide', action='store_true', help='Use a 1280×900 demo for selected groups')
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/line-desktop')
    parser.add_argument('--screenshots', type=Path, help='Save group and intermediate surface screenshots in this directory')
    args = parser.parse_args()
    if args.all and (args.group or args.wide):
        parser.error('--all supplies the groups and window sizes')
    selected = args.group or ['smoke']
    if args.all:
        matrix = [(980, [g for g in GROUPS if g not in ('signed-out', 'missing-module') and g not in SCENES]),
                  (1280, list(WIDE_GROUPS)), (420, ["focus"])]
        matrix += [(980, [g]) for g in (*SCENES, 'signed-out', 'missing-module')]
    else:
        regular = [g for g in selected if g not in ('signed-out', 'missing-module') and g not in SCENES]
        matrix = ([(1280 if args.wide else 980, regular)] if regular else []) + [(1280 if args.wide else 980, [g]) for g in selected if g in ('signed-out', 'missing-module') or g in SCENES]
    seed = args.seed if args.seed is not None else secrets.randbits(32)
    print(f'Seed: {seed}', flush=True)
    if args.all:
        rng = random.Random(seed)
        for _, groups in matrix:
            rng.shuffle(groups)
        rng.shuffle(matrix)
    started = time.monotonic()
    failures = 0
    for width, groups in matrix:
        # Smoke probes startup idle before reset can stop a mistakenly running timer.
        if 'smoke' in groups:
            groups = ['smoke'] + [group for group in groups if group != 'smoke']
        args.wide = width == 1280
        size_suffix = "-narrow" if width < 740 else "-wide" if args.wide else ""
        # Ask the OS for an unused port; readiness is determined only by initialize.
        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1', 0))
            args.port = reservation.getsockname()[1]
        flags = ['--demo-size', f'{width}x{900 if args.wide else 720}']
        if args.dark:
            flags += ['--demo-dark']
        if groups[0] in SCENES:
            flags += ['--snapshot-scene', SCENES[groups[0]]]
        if groups[0] in ('signed-out', 'missing-module'):
            flags += ['--demo', '--snapshot-scene', 'signed-out']
        if groups == ['missing-module']:
            flags += ['--snapshot-case', 'missing-module']
        with tempfile.TemporaryFile(dir=UI_TEMP, mode='w+') as log:
            launch_started = time.monotonic()
            completed = 0
            try:
                with launch_demo(args.binary, args.port, flags, log) as process:
                    demo_directory = process.demo_directory
                    connect(process)
                    children = Path(f"/proc/{process.pid}/task/{process.pid}/children").read_text().split()
                    demo_pid = next(int(pid) for pid in children if Path(f"/proc/{pid}/comm").read_text().strip() == "line-desktop")
                    for index, group in enumerate(groups):
                        current_group = group + size_suffix
                        group_started = launch_started if index == 0 else time.monotonic()
                        try:
                            if group == 'smoke':
                                check_idle()
                            if group not in ('signed-out', 'missing-module') and group not in SCENES:
                                reset_demo()
                            if group == 'focus' and width < 740:
                                check_narrow_chats_focus()
                            else:
                                GROUPS[group]()
                            screenshot('complete')
                            print(f'PASS {current_group} {time.monotonic() - group_started:.3f}s', flush=True)
                        except Exception:
                            failures += 1
                            print(f'FAIL {current_group} {time.monotonic() - group_started:.3f}s', flush=True)
                            traceback.print_exc()
                        completed += 1
            except WrongInstanceError:
                traceback.print_exc()
                return 1
            except Exception:
                failures += 1
                for group in groups[completed:]:
                    label = group + size_suffix
                    print(f'FAIL {label} {time.monotonic() - launch_started:.3f}s', flush=True)
                traceback.print_exc()
                log.seek(0)
                print(log.read(), file=sys.stderr)
    print(f'{"FAIL" if failures else "PASS"} total {time.monotonic() - started:.3f}s', flush=True)
    return int(bool(failures))


if __name__ == '__main__':
    raise SystemExit(main())
