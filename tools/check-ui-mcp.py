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

from run_ui_debug import launch_demo

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
        if "Invalid element handle" in message or "refers to element that was destroyed" in message:
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
    return call("get_element_properties", elementHandle=handles[0])

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

def check_detail_stack():
    def focused(label, role="Button"):
        assert "Focused" in element(label, role).get("accessibleDescription", ""), f"focus not restored to {label}: {[item.get('accessibleLabel') for item in tree() if 'Focused' in item.get('accessibleDescription','')]}"
    click("Friends")
    search=element("Search friends and groups", "TextInput")
    call("set_element_value",elementHandle=search["handle"],value="")
    click("Open profile: Weekend plans")
    wait_element("Group info", "Text")
    assert any(item.get("accessibleLabel")=="Weekend plans" for item in tree()), "group info is not bound to its chat"
    click("Members (25)")
    wait_element("Search members", "TextInput")
    set_value("Search members", "葵")
    click("Open member: 葵")
    wait_element("Profile", "Text")
    click("Rename")
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

def check_smoke():
    global draft, before_shortcut
    show_chats()
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


def check_focus():
    def focus_label():
        focused = [item for item in tree() if item.get("accessibleDescription", "").startswith("Focused")]
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
        return ["Navigation", search_label] + (["All", "New chat"] if list_label == "Conversations" else []) + (recovery if narrow else []) + [list_label]

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
                expected += ["葵", "More"] + recovery + ["Message history", "Message draft"]
                if element("Stickers", "Button").get("accessibleEnabled"): expected.append("Stickers")
                if element("Attach", "Button").get("accessibleEnabled"): expected.append("Attach")
                if element("Send", "Button").get("accessibleEnabled"): expected.append("Send")
            check_chain(f"{surface} {suffix}", expected, (search_label, "TextInput"))
            set_value(search_label, "葵")
            check_chain(f"{surface} search clear {suffix}", expected[:2] + ["Clear search"] + expected[2:], (search_label, "TextInput"))
            click("Clear search")
        click("Chats")
        for chat in ("葵", "Weekend plans"):
            click(chat)
            set_value("Message draft", "Focus check")
            expected = ["Back"] if narrow else ["Navigation", "Find a conversation", "All", "New chat", "Conversations"]
            expected.append(chat)
            if chat == "Weekend plans": expected.append(element("Members", "Button")["accessibleLabel"])
            expected += ["More"] + recovery + ["Message history", "Message draft"]
            if element("Stickers", "Button").get("accessibleEnabled"): expected.append("Stickers")
            if element("Attach", "Button").get("accessibleEnabled"): expected.append("Attach")
            if element("Send", "Button").get("accessibleEnabled"): expected.append("Send")
            check_chain(f"conversation {chat} {suffix}", expected, ("Message draft", "TextInput"))
            if chat == "Weekend plans":
                click("Members")
                layer = ["Close details", "Search members", "Group members"]
                prefix = expected if panel else [] if narrow else ["Navigation", "Find a conversation", "All", "New chat", "Conversations"]
                check_chain(f"members {layer_kind} {suffix}", prefix + layer, ("Close details", "Button"))
                set_value("Search members", "葵")
                check_chain(f"members search clear {suffix}", prefix + layer[:2] + ["Clear search"] + layer[2:], ("Close details", "Button"))
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
            prefix += ["Weekend plans", element("Members", "Button")["accessibleLabel"], "More"] + recovery + ["Message history", "Message draft"]
            if element("Stickers", "Button").get("accessibleEnabled"): prefix.append("Stickers")
            if element("Attach", "Button").get("accessibleEnabled"): prefix.append("Attach")
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


def check_compose():
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
    open_reactions()
    wait_for(lambda: element("LOVE", "Button").get("accessibleItemSelected"), "own reaction was not highlighted")
    screenshot("reactions")
    click("LOVE")
    open_reactions()
    wait_for(lambda: not element("LOVE", "Button").get("accessibleItemSelected"), "repeat pick did not cancel")
    key("\x1b")
    body = next(item for item in tree() if item.get("accessibleRole") == "TextInput" and item.get("accessibleValue") == before_shortcut)
    call("click_element", elementHandle=body["handle"])
    for label in ("React", "Reply"):
        key("\t")
        assert any(item.get("accessibleLabel") == label and item.get("accessibleDescription") == "Focused" for item in tree()), f"body → React → Reply lost focus at {label}"
    key("\t")
    assert element("Message draft", "TextInput").get("accessibleDescription") == "Focused message draft", "Reply did not tab to composer"
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
    assert element("Message draft", "TextInput").get("accessibleDescription") == "Focused message draft"
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
    click("Send sticker 1")
    wait_for(lambda: any("Sticker" in item.get("accessibleLabel", "") and item.get("accessibleRole") in ("Text", "Image") for item in tree()), "sticker did not reach history")
    assert not any(item.get("accessibleLabel") == "Send sticker 1" for item in tree()), "picker stayed open after send"
    click("Stickers")
    wait_for(lambda: element("Sticker preview 1", "Image").get("accessibleDescription") == "Loaded", "cached picker did not reopen")
    key("\x1b")
    assert not any(item.get("accessibleLabel") == "Send sticker 1" for item in tree()), "Escape did not close sticker picker"
    show_chats()
    click("New chat")
    wait_element("Find friends for new chat", "TextInput")
    screenshot("new-chat")
    click("新しい友だち 🌸", "Checkbox")
    click("Chat")
    wait_for(lambda: any(item.get("accessibleLabel") == "新しい友だち 🌸" and item.get("accessibleRole") == "Button" for item in tree()), "friend conversation did not open")
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
    with tempfile.TemporaryDirectory(prefix="line-attach-ui-") as directory:
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
        injection = element("Attachment paths", "TextInput")
        call("set_element_value", elementHandle=injection["handle"], value=json.dumps([str(photo), str(file), str(removed)]))
        click("Preview attachments")
        wait_element("Send 3", "Button")
        screenshot("attachments")
        click("Remove Remove.txt")
        wait_element("Send 2", "Button")
        assert not any(item.get("accessibleLabel", "").startswith("Uploading") for item in tree()), "selection uploaded before Send"
        click("Send 2")
        wait_for(lambda: any(item.get("accessibleLabel", "").startswith("Uploading ") and item.get("accessibleLabel") not in ("Uploading 0%", "Uploading 100%") for item in tree()), "upload progress did not appear")
        screenshot("uploading")
        wait_for(lambda: any(item.get("accessibleValue") == "Notes.txt" for item in tree()) and not any(item.get("accessibleLabel", "").startswith("Uploading") or item.get("accessibleLabel") == "Sending" for item in tree()), "attachment rows did not finish")


def check_scenario_keys():
    click("Message draft", "TextInput")
    for dismiss in ("escape", "close"):
        key("\uf70f")
        assert scenario_element("Close demo scenarios").get("accessibleDescription") == "Focused"
        if dismiss == "escape": key("\x1b")
        else: click("Close demo scenarios")
        assert element("Message draft", "TextInput").get("accessibleDescription") == "Focused message draft", "scenarios did not restore the composer focus"
    def reverse_tab():
        key("\x10", "Press")
        key("\t")
        key("\x10", "Release")
    for closing_key in ("\x1b", " "):
        key("\uf70f")
        key("\t")
        assert scenario_element("Long history").get("accessibleDescription") == "Focused", "Tab from Close did not focus History"
        reverse_tab()
        assert scenario_element("Close demo scenarios").get("accessibleDescription") == "Focused"
        reverse_tab()
        assert scenario_element("Offline").get("accessibleDescription") == "Focused", "reverse Tab focused the wrong scenario"
        key(closing_key)
        assert not call("find_elements_by_id", windowHandle=window, elementsId="DemoScenarios::scenario-panel").get("elementHandles", []), "keyboard activation did not dismiss scenarios"
        key("\uf70f")
        assert scenario_element("Close demo scenarios").get("accessibleDescription") == "Focused"
        key("\t")
        assert scenario_element("Long history").get("accessibleDescription") == "Focused", "stale last-scenario focus trapped Tab after reopening"
        key("\x1b")
    key("\uf70f")
    call("click_element", elementHandle=scenario_action("Media failures")["handle"])
    reverse_tab()
    assert scenario_element("Media cancelled").get("accessibleDescription") == "Focused", "reverse Tab did not focus the final media scenario"
    key(" ")
    key("\uf70f")
    key("\t")
    assert scenario_element("Long history").get("accessibleDescription") == "Focused", "stale media focus trapped Tab after reopening"
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
    assert element("Sign in again", "Button").get("accessibleDescription") == "Focused"
    key("\n")
    assert any("Signing in here signs LINE out in Chrome." in item.get("accessibleLabel", "") for item in tree())
    assert element("Start", "Button").get("accessibleDescription") == "Focused"
    key("\x1b")
    assert element("Sign in again", "Button").get("accessibleDescription") == "Focused"
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == before, "sign-in dismissal changed the draft"
    key("\t")
    assert element("Message history", "List").get("accessibleDescription") == "Focused history"
    click("Sign in again", "Button")
    click("Start", "Button")
    wait_for(lambda: any("LINE refused sign-in" in item.get("accessibleLabel", "") for item in tree()), "synthetic sign-in did not report its failure", timeout=3)
    key("\x1b")
    assert element("Sign in again", "Button").get("accessibleDescription") == "Focused"
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == before, "failed sign-in changed the draft"


def check_layers():
    demo_scenario("Protocol fixtures")
    # Public OA metadata assets use the same offline media paths as other photos.
    click("Open photo", "Button")
    wait_element("Photo in viewer", "Image")
    key("\x1b")
    demo_scenario("Live reaction")
    wait_element("👍", "Text")
    wait_element("1", "Text")

    def unsend_menu(body):
        message = next(item for item in tree() if body in item.get("accessibleLabel", "") and item.get("accessibleRole") in ("Text", "TextInput"))
        call("click_element", elementHandle=message["handle"], button="Right")
        click("Unsend", "Text")
        wait_element("Unsend message?", "Text")
        wait_element("It will be removed for everyone.", "Text")
        assert element("Cancel", "Button").get("accessibleDescription", "") in ("", "Focused")
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
        assert all(dialog["absolutePosition"][axis] > 0 and abs(dialog["absolutePosition"][axis] * 2 + dialog["size"][extent] - properties["size"][extent] / scale) <= 1 for axis, extent in (("x", "width"), ("y", "height"))), "wide Settings did not form a centered dialog"
    wait_element("Enter sends", "Checkbox")
    assert call("get_element_properties", elementHandle=close_settings["handle"]).get("accessibleDescription") == "Focused", "Settings did not focus its close control"
    key("\t")
    assert element("Settings section", "TabList").get("accessibleDescription") == "Focused", "Settings navigation did not describe focus"
    for _ in range(12):
        key("\t")
    assert wait_element("Message draft", "TextInput").get("accessibleDescription") != "Focused message draft", "Settings let Tab escape to the draft"
    key("\x1b")
    assert element("Navigation", "Button").get("accessibleDescription") == "Focused navigation", "Settings did not restore rail focus"
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == before
    click("Friends", "Button")
    call("set_element_value", elementHandle=handle("Search friends and groups", "TextInput"), value="葵")
    click("Open profile: 葵", "Button")
    assert any(item.get("accessibleLabel") == "Message draft" for item in tree()) == args.wide, "profile conversation visibility disagrees with layout"
    key("\x1b")
    assert element("Friends and groups", "List").get("accessibleDescription") == "Focused friends and groups", "profile did not restore its list opener"
    click("Chats", "Button")
    demo_scenario("Photo preview")
    click("Open photo", "Button")
    click("Next", "Button")
    wait_for(lambda: any("800 × 1200" in item.get("accessibleLabel", "") for item in tree()), "Next left the loaded photo scope")
    click("Previous", "Button")
    wait_for(lambda: any("1200 × 800" in item.get("accessibleLabel", "") for item in tree()), "Previous did not load the first photo")
    click("Original size")
    assert element("Photo in viewer", "Image")["size"]["width"] >= 1199, "original-size view only enlarged the thumbnail"
    click("Fit")
    key("1")
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
    click("Fit")
    fitted = element("Photo in viewer", "Image")["size"]
    assert fitted["width"] <= properties["size"]["width"] + 1 and fitted["height"] <= properties["size"]["height"] + 1, "Fit did not restore a bounded view"
    key("1")
    key("0")
    fitted = element("Photo in viewer", "Image")["size"]
    assert fitted["width"] <= properties["size"]["width"] + 1 and fitted["height"] <= properties["size"]["height"] + 1, "0 did not fit the photo"
    click("Save photo", "Button")
    saved = wait_for(lambda: [item.get("accessibleLabel", "") for item in tree() if item.get("accessibleLabel", "").startswith("Saved to ")], "Save did not report a completed export")
    exported = Path(saved[0].removeprefix("Saved to "))
    assert exported.is_relative_to(demo_directory / "exports"), "preview export escaped its isolated folder"
    assert exported.read_bytes().startswith(b"\x89PNG\r\n\x1a\n")
    exported.unlink()
    key("\x1b")
    assert any(item.get("accessibleLabel") == "Open photo" and item.get("accessibleDescription") == "Focused" for item in tree()), "viewer did not restore the photo opener"
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
    for _ in range(2):
        key("\x10", "Press")
        key("\t")
        key("\x10", "Release")
        history = element("Message history", "List")
        if history.get("accessibleDescription") == "Focused history":
            break
    assert history.get("accessibleDescription") == "Focused history", "reverse Tab entered a message"
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
        call("click_element", elementHandle=handle("Message draft", "TextInput"))
        for label, role in [("Return to message", "Button"), (latest["accessibleLabel"], "Button"), ("Message history", "List")]:
            key("\x10", "Press")
            key("\t")
            key("\x10", "Release")
            assert element(label, role).get("accessibleDescription", "").startswith("Focused"), f"reverse Tab skipped {label}"
        assert element("Message history", "List").get("accessibleDescription") == "Focused history", "reverse Tab skipped a history pill"
        for label, role in [(latest["accessibleLabel"], "Button"), ("Return to message", "Button"), ("Message draft", "TextInput")]:
            key("\t")
            assert element(label, role).get("accessibleDescription", "").startswith("Focused"), f"forward Tab skipped {label}"
        assert wait_element("Message draft", "TextInput").get("accessibleDescription") == "Focused message draft", "history/Latest/Return/composer Tab order changed"
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
    assert wait_element("Message draft", "TextInput").get("accessibleValue") == original, "rich copy lost emoji fallback or newlines"
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


GROUPS = {
    'smoke': check_smoke,
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
    markers = call('find_elements_by_id', windowHandle=window, elementsId='AppWindow::demo-reset-marker').get('elementHandles', [])
    if len(markers) != 1:
        raise WrongInstanceError('connected instance is not an automation demo')
    marker = call('get_element_properties', elementHandle=markers[0])
    if marker.get('accessibleLabel') != 'Demo automation':
        raise WrongInstanceError('connected instance is not a demo')
    if marker.get('accessibleValue') != process.demo_token + ':0':
        raise WrongInstanceError('MCP answered from the wrong process')
    connected = True


def reset_demo():
    global draft, before_shortcut
    for _ in range(4):
        key('\x1b')
    key("\uf70f")
    generation = bounds_by_id('AppWindow::demo-reset-marker')['accessibleValue']
    click('Reset demo')
    wait_for(lambda: bounds_by_id('AppWindow::demo-reset-marker')['accessibleValue'] != generation, 'demo reset did not complete')
    wait_element('葵')
    smoke_chat('葵')
    draft = '日本語の下書き\nDraft with emoji 🐈'
    before_shortcut = draft + '\n'
    set_value('Message draft', draft)


def main():
    global args, current_group, demo_directory
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--seed', type=int, help='Reproduce the randomized --all group order')
    parser.add_argument('--all', action='store_true', help='Run the normal and wide behavioural matrix')
    parser.add_argument('--group', choices=GROUPS, action='append', help='Run a group (repeatable; defaults to smoke)')
    parser.add_argument('--wide', action='store_true', help='Use a 1280×900 demo for selected groups')
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/line-desktop')
    parser.add_argument('--screenshots', type=Path, help='Save group and intermediate surface screenshots in this directory')
    args = parser.parse_args()
    if args.all and (args.group or args.wide):
        parser.error('--all supplies the groups and both window sizes')
    selected = args.group or ['smoke']
    if args.all:
        matrix = [(False, [g for g in GROUPS if g not in ('signed-out', 'missing-module')]),
                  (True, list(WIDE_GROUPS))]
        matrix += [(False, ['signed-out']), (False, ['missing-module'])]
    else:
        regular = [g for g in selected if g not in ('signed-out', 'missing-module')]
        matrix = ([(args.wide, regular)] if regular else []) + [(args.wide, [g]) for g in selected if g in ('signed-out', 'missing-module')]
    seed = args.seed if args.seed is not None else secrets.randbits(32)
    print(f'Seed: {seed}', flush=True)
    if args.all:
        rng = random.Random(seed)
        for _, groups in matrix:
            rng.shuffle(groups)
        rng.shuffle(matrix)
    started = time.monotonic()
    failures = 0
    for wide, groups in matrix:
        args.wide = wide
        # Ask the OS for an unused port; readiness is determined only by initialize.
        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1', 0))
            args.port = reservation.getsockname()[1]
        flags = ['--demo-size', '1280x900' if wide else '980x720']
        if groups[0] in ('signed-out', 'missing-module'):
            flags += ['--demo-signed-out']
        if groups == ['missing-module']:
            flags += ['--snapshot-case', 'missing-module']
        with tempfile.TemporaryFile(mode='w+') as log:
            launch_started = time.monotonic()
            completed = 0
            try:
                with launch_demo(args.binary, args.port, flags, log) as process:
                    demo_directory = process.demo_directory
                    connect(process)
                    for index, group in enumerate(groups):
                        current_group = group + ('-wide' if wide else '')
                        group_started = launch_started if index == 0 else time.monotonic()
                        try:
                            if group not in ('signed-out', 'missing-module'):
                                reset_demo()
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
                    label = group + ('-wide' if wide else '')
                    print(f'FAIL {label} {time.monotonic() - launch_started:.3f}s', flush=True)
                traceback.print_exc()
                log.seek(0)
                print(log.read(), file=sys.stderr)
    print(f'{"FAIL" if failures else "PASS"} total {time.monotonic() - started:.3f}s', flush=True)
    return int(bool(failures))


if __name__ == '__main__':
    raise SystemExit(main())
