#!/usr/bin/env python3
"""Check single app-owned clear graphics in an owned offline preview in light/dark previews.

Launch with tools/run-ui-debug.py, --demo-text-size 15 or 24,
--demo-color-scheme light or dark and the desired --demo-size; pass the wrapper's unique --port. Screenshots require Pillow.
"""
import argparse
import base64
import io
import json
import math
from pathlib import Path
import urllib.request

from PIL import Image, ImageChops

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--port", type=int, required=True)
parser.add_argument("--artifacts", type=Path, required=True)
parser.add_argument("--text-size", type=int, choices=[15, 24], default=15)
parser.add_argument("--color-scheme", choices=["light", "dark"], default="light")
args = parser.parse_args()
args.artifacts.mkdir(parents=True, exist_ok=True)
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


rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "line-search-pixel-check", "version": "1"}})
window = call("list_windows")["windowHandles"][0]
root = call("get_window_properties", windowHandle=window)["rootElementHandle"]


def tree():
    result = call("get_element_tree", elementHandle=root, maxElements=700)
    assert not result.get("truncated"), "window exceeded its tree bound"
    return result["elements"]


def element(label, role="Button"):
    return next(row for row in tree() if row.get("accessibleRole") == role and row.get("accessibleLabel") == label)


def click(label, role="Button"):
    call("click_element", elementHandle=element(label, role)["handle"])


def key(text):
    call("dispatch_key_event", windowHandle=window, text=text, eventType="PressAndRelease")


def clear_graphics(image, box, scale):
    crop = image.crop(box).convert("RGB")
    background = max(crop.getcolors(crop.width * crop.height))[1]
    pixels = crop.load()
    ink = set()
    for y in range(crop.height):
        for x in range(crop.width):
            color = pixels[x, y]
            if max(color) - min(color) < 48 and sum(abs(a - b) for a, b in zip(color, background)) > 150:
                ink.add((x, y))
    graphics = []
    while ink:
        pending = [ink.pop()]
        component = []
        while pending:
            x, y = pending.pop()
            component.append((x, y))
            for dx in [-1, 0, 1]:
                for dy in [-1, 0, 1]:
                    neighbor = (x + dx, y + dy)
                    if neighbor in ink:
                        ink.remove(neighbor)
                        pending.append(neighbor)
        left = min(x for x, y in component)
        top = min(y for x, y in component)
        width = max(x for x, y in component) - left + 1
        height = max(y for x, y in component) - top + 1
        # Both Fluent's small dismiss glyph and the app's SVG fit these bounds.
        if 6 * scale <= width <= 24 * scale and 6 * scale <= height <= 24 * scale and 0.6 <= width / height <= 1.67:
            graphics.append([left, top, width, height])
    return graphics


def screenshot():
    result = rpc("tools/call", {"name": "take_screenshot", "arguments": {"windowHandle": window}})
    assert not result.get("isError"), result
    encoded = next(part["data"] for part in result["content"] if part["type"] == "image")
    return Image.open(io.BytesIO(base64.b64decode(encoded))).convert("RGB")


def typed(row, name):
    return any(base.get("typeName") == name for base in row.get("typeNamesAndIds", []))


def capture(destination, search, state, query):
    field = element(search, "TextInput")
    rows = tree()
    wrapper = next(row for row in rows if typed(row, "SearchField"))
    clip = next(row for row in rows if any(base.get("id", "").endswith("::editor-clip") for base in row.get("typeNamesAndIds", [])))
    clear = element("Clear search") if query else None
    assert field.get("accessibleValue", "") == query
    assert abs(field["size"]["height"] - max(40, args.text_size * 1.4 + 8)) < 0.1, "launch with the matching --demo-text-size"
    editor = call("get_element_tree", elementHandle=field["handle"], maxElements=64)
    assert not editor.get("truncated"), "search exceeded its tree bound"
    focused = state.endswith("focused") and not state.endswith("button-focused")
    assert (field.get("accessibleDescription") == "Focused search") == focused
    edge = clip["absolutePosition"]["x"] + clip["size"]["width"]
    viewport = next(row for row in editor["elements"] if typed(row, "LineEditBase"))
    assert abs(viewport["absolutePosition"]["x"] + viewport["size"]["width"] - edge) < 0.1, "native viewport does not end at the clip"
    native = [row for row in editor["elements"] if typed(row, "LineEditClearIcon")]
    assert all(row["absolutePosition"]["x"] >= edge for row in native), "native clear is inside the clip"
    if clear:
        assert clear["size"]["width"] >= 24 and clear["size"]["height"] >= 24, "Clear target too small"
        assert 2 <= clear["absolutePosition"]["x"] - edge <= 4, "missing text/Clear gap"
        assert clear["absolutePosition"]["x"] + clear["size"]["width"] <= wrapper["absolutePosition"]["x"] + wrapper["size"]["width"]
    else:
        assert not any(row.get("accessibleLabel") == "Clear search" for row in rows)
    if len(query) > 40:
        body = next(row for row in editor["elements"] if typed(row, "TextInput"))
        assert body["absolutePosition"]["x"] < viewport["absolutePosition"]["x"], "long query did not scroll"
        assert edge - 2 <= body["absolutePosition"]["x"] + body["size"]["width"] <= edge, "tail/caret is clipped"
    image = screenshot()
    window_size = call("get_element_properties", elementHandle=root)["size"]
    scale = image.width / window_size["width"]
    x, y = (wrapper["absolutePosition"][axis] for axis in ["x", "y"])
    right = x + wrapper["size"]["width"]
    bottom = y + wrapper["size"]["height"]
    box = (math.floor(x * scale), math.floor(y * scale), math.ceil(right * scale), math.ceil(bottom * scale))
    stem = args.artifacts / f"{destination.lower()}-{int(window_size['width'])}-t{args.text_size}-{args.color_scheme}-{state}"
    image.save(stem.with_suffix(".png"))
    image.crop(box).resize(((box[2] - box[0]) * 3, (box[3] - box[1]) * 3)).save(stem.with_name(stem.name + "-field").with_suffix(".png"))
    # These rows avoid text and the inset button. Exact continuity catches both
    # compositing seams and a focus underline cut off beneath Clear.
    for name, row_y in [("fill", math.floor((y + 2) * scale)), ("top", round(y * scale)), ("bottom", math.ceil(bottom * scale) - 1), ("underline", round(bottom * scale) - round(2 * scale))]:
        colors = [image.getpixel((pixel_x, row_y)) for pixel_x in range(math.ceil((x + 12) * scale), math.floor((right - 8) * scale))]
        assert all(max(abs(a - b) for a, b in zip(color, colors[0])) <= 1 for color in colors), f"{stem}: discontinuous {name}: {set(colors)}"
    fill = image.getpixel((round((x + 16) * scale), math.floor((y + 2) * scale)))
    assert (sum(fill) < 384) == (args.color_scheme == "dark"), "launch with the matching --demo-color-scheme"
    if clear:
        icon_box = (math.floor(min([clear["absolutePosition"]["x"]] + [row["absolutePosition"]["x"] for row in native]) * scale), math.floor((y + 8) * scale), math.ceil((clear["absolutePosition"]["x"] + clear["size"]["width"]) * scale), math.ceil((bottom - 8) * scale))
        graphics = clear_graphics(image, icon_box, scale)
        assert len(graphics) == 1, f"{stem}: expected one clear graphic, got {graphics}"
        image.crop(icon_box).resize(((icon_box[2] - icon_box[0]) * 4, (icon_box[3] - icon_box[1]) * 4)).save(stem.with_name(stem.name + "-icon").with_suffix(".png"))
    print(f"PASS: {stem.name}: continuous Fluent fill/strokes, clipped native icon, {'one clear graphic' if query else 'no Clear'}", flush=True)
    return image, box


# This marker exists exclusively in --demo; never touch an account window.
key("\uf70f")
marker = element("Synthetic interaction scenarios", "Text")
assert any(base.get("id", "").endswith("::demo-marker") for base in marker.get("typeNamesAndIds", [])), "refusing a window without the compiled demo-only marker"
click("Close demo scenarios")
if any(row.get("accessibleLabel") == "Back" and row.get("accessibleRole") == "Button" for row in tree()):
    click("Back")
for destination, search, listing in [("Chats", "Find a conversation", "Conversations"), ("Friends", "Search friends and groups", "Friends and groups")]:
    click(destination)
    click(search, "TextInput")
    call("move_pointer", windowHandle=window, position={"x": 1, "y": 1})
    capture(destination, search, "empty-focused", "")
    key("\t")
    assert element(listing, "List").get("accessibleDescription") == ("Focused conversations" if destination == "Chats" else "Focused friends and groups"), "empty search Tab skipped the list"
    capture(destination, search, "empty-blurred", "")
    call("hover_element", elementHandle=element(search, "TextInput")["handle"])
    capture(destination, search, "empty-hovered", "")
    click(search, "TextInput")
    call("move_pointer", windowHandle=window, position={"x": 1, "y": 1})
    key("Q")
    capture(destination, search, "query-focused", "Q")
    key("\t")
    assert element("Clear search").get("accessibleDescription") == "Focused"
    capture(destination, search, "query-button-focused", "Q")
    key("\t")
    assert element("Clear search").get("accessibleDescription") != "Focused"
    assert element(listing, "List").get("accessibleDescription") == ("Focused conversations" if destination == "Chats" else "Focused friends and groups"), "Clear Tab skipped the list"
    plain, _ = capture(destination, search, "query-blurred", "Q")
    call("hover_element", elementHandle=element(search, "TextInput")["handle"])
    capture(destination, search, "query-hovered", "Q")
    call("hover_element", elementHandle=element("Clear search")["handle"])
    hovered, _ = capture(destination, search, "query-clear-hovered", "Q")
    clear = element("Clear search")
    scale = plain.width / call("get_element_properties", elementHandle=root)["size"]["width"]
    box = tuple(round(value * scale) for value in [clear["absolutePosition"]["x"], clear["absolutePosition"]["y"], clear["absolutePosition"]["x"] + clear["size"]["width"], clear["absolutePosition"]["y"] + clear["size"]["height"]])
    difference = ImageChops.difference(plain.crop(box), hovered.crop(box)).convert("L")
    assert difference.width * difference.height - difference.histogram()[0] > 100 * scale * scale, "Clear lost hover feedback"
    click("Clear search")
    assert element(search, "TextInput").get("accessibleValue", "") == ""
    assert element(search, "TextInput").get("accessibleDescription") == "Focused search"
    key("Q")
    key("\x1b")
    assert element(search, "TextInput").get("accessibleValue", "") == ""
    call("move_pointer", windowHandle=window, position={"x": 1, "y": 1})
    query = "0123456789" * 6 + "TAIL66"
    key(query)
    capture(destination, search, "long-focused", query)
    key("\t")
    key("\t")
    capture(destination, search, "long-blurred", query)
    click("Clear search")
    assert element(search, "TextInput").get("accessibleValue", "") == ""
print("PASS: native editing, one persistent Clear with hover feedback and a 32px target, pointer clear/refocus, Tab and Escape")


click("Chats")
key("\uf70f")  # F12
try:
    offline = element("Offline")
except StopIteration:
    close = element("Close demo scenarios")
    position = {"x": close["absolutePosition"]["x"] + close["size"]["width"] / 2, "y": close["absolutePosition"]["y"] + close["size"]["height"] + 60}
    for delta in (10000, -320, -320, -320):
        call("dispatch_pointer_scroll", windowHandle=window, position=position, deltaY=delta)
        try:
            offline = element("Offline")
            break
        except StopIteration:
            pass
    else:
        raise AssertionError("Offline scenario was not reachable") from None
call("click_element", elementHandle=offline["handle"])
narrow = call("get_element_properties", elementHandle=root)["size"]["width"] < 740


def shift_tab():
    call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Press")
    key("\t")
    call("dispatch_key_event", windowHandle=window, text="\x10", eventType="Release")


for query in ("", "a"):
    field = element("Find a conversation", "TextInput")
    call("set_element_value", elementHandle=field["handle"], value=query)
    click("Find a conversation", "TextInput")
    key("\t")
    if query:
        assert element("Clear search").get("accessibleDescription") == "Focused"
        shift_tab()
        assert element("Find a conversation", "TextInput").get("accessibleDescription") == "Focused search", "reverse Tab from Clear skipped the editor"
        key("\t")
        assert element("Clear search").get("accessibleDescription") == "Focused"
        key("\t")
    if not narrow:
        assert element("Conversations", "List").get("accessibleDescription") == "Focused conversations", "forward Tab skipped the list"
        shift_tab()
        assert element("Find a conversation", "TextInput").get("accessibleDescription") == "Focused search", "reverse Tab from list skipped search"
        key("\t")
        if query:
            key("\t")
        assert element("Conversations", "List").get("accessibleDescription") == "Focused conversations"
        key("\t")
    assert element("Retry connection").get("accessibleDescription") == "Focused", "forward Tab skipped the Offline strip"
    shift_tab()
    if narrow:
        assert element("Find a conversation", "TextInput").get("accessibleDescription") == "Focused search", "reverse Tab from strip skipped search"
        shift_tab()
        assert element("Navigation").get("accessibleDescription") == "Focused navigation", "reverse Tab from search entered an unlabeled stop"
        key("\t")
        assert element("Find a conversation", "TextInput").get("accessibleDescription") == "Focused search"
        key("\t")
        if query:
            key("\t")
    else:
        assert element("Conversations", "List").get("accessibleDescription") == "Focused conversations", "reverse Tab from strip skipped the list"
        key("\t")
    assert element("Retry connection").get("accessibleDescription") == "Focused"
    key("\t")
    target, role, description = ("Conversations", "List", "Focused conversations") if narrow else ("Message history", "List", "Focused history")
    assert element(target, role).get("accessibleDescription") == description, "forward Tab from strip skipped its content"
    assert element("Find a conversation", "TextInput").get("accessibleValue", "") == query, "Tab traversal changed the query"
    if query:
        click("Find a conversation", "TextInput")
        key("\t")
        assert element("Clear search").get("accessibleDescription") == "Focused"
        key("\n")
        assert element("Find a conversation", "TextInput").get("accessibleValue", "") == "", "Enter on Clear did not clear the query"
        assert element("Find a conversation", "TextInput").get("accessibleDescription") == "Focused search"
print("PASS: Offline empty/query search, Clear, list and strip forward/reverse Tab paths; no unlabeled stop")
field = element("Find a conversation", "TextInput")
call("set_element_value", elementHandle=field["handle"], value="Weekend")
if narrow:
    strip = element("Connection needs attention", "Banner")
    row = next(row for row in tree() if row.get("accessibleRole") == "Button" and row.get("accessibleLabel", "").startswith("Weekend plans, "))
    assert row["absolutePosition"]["y"] >= strip["absolutePosition"]["y"] + strip["size"]["height"] - 0.5, "Offline strip covered the queried row"
screenshot().save(args.artifacts / f"chats-offline-query-{'narrow' if narrow else 'wide'}-t{args.text_size}-{args.color_scheme}.png")
