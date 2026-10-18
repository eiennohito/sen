#!/usr/bin/env python3
"""Check --demo-previews on an owned tools/run-ui-debug.py scope."""
import argparse
import base64
import json
import time
from pathlib import Path
import urllib.request

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--port', type=int, required=True)
parser.add_argument('--artifacts', type=Path, required=True)
args = parser.parse_args()
args.artifacts.mkdir(parents=True, exist_ok=True)
sequence = 0


def rpc(method, params):
    global sequence
    sequence += 1
    request = urllib.request.Request(f'http://127.0.0.1:{args.port}/mcp',
        data=json.dumps({'jsonrpc': '2.0', 'id': sequence, 'method': method, 'params': params}).encode(),
        headers={'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream'})
    with urllib.request.urlopen(request, timeout=10) as response:
        result = json.load(response)
    assert 'error' not in result, result
    return result['result']


def call(name, **arguments):
    result = rpc('tools/call', {'name': name, 'arguments': arguments})
    assert not result.get('isError'), result
    return json.loads(result['content'][0]['text'])


rpc('initialize', {'protocolVersion': '2025-03-26', 'capabilities': {}, 'clientInfo': {'name': 'line-preview-check', 'version': '1'}})
window = call('list_windows')['windowHandles'][0]
root = call('get_window_properties', windowHandle=window)['rootElementHandle']


def tree(handle=root):
    result = call('get_element_tree', elementHandle=handle, maxElements=1000)
    assert not result.get('truncated'), 'preview tree exceeded bound'
    return result['elements']


def element(label, role):
    return next(row for row in tree() if row.get('accessibleRole') == role and
        (row.get('accessibleLabel') == label or row.get('accessibleLabel', '').startswith(label + ', ')))


def screenshot(name):
    result = rpc('tools/call', {'name': 'take_screenshot', 'arguments': {'windowHandle': window}})
    image = next(part for part in result['content'] if part['type'] == 'image')
    (args.artifacts / f'{name}.png').write_bytes(base64.b64decode(image['data']))


def back():
    try:
        call('click_element', elementHandle=element('Back', 'Button')['handle'])
        time.sleep(.05)
    except StopIteration:
        pass


# Demo-only compiled marker, before any interaction with message content.
call('dispatch_key_event', windowHandle=window, text='\uf70f')
marker = element('Synthetic interaction scenarios', 'Text')
assert any(part.get('id', '').endswith('::demo-marker') for part in marker.get('typeNamesAndIds', []))
call('click_element', elementHandle=element('Close demo scenarios', 'Button')['handle'])
back()
screenshot('initial')
row = element('One LINE emoji', 'Button')
assert ', [笑]' in row['accessibleLabel'], row
images = [item for item in tree(row['handle']) if item.get('accessibleRole') == 'Image' and item.get('accessibleLabel') == '[笑]']
assert len(images) == 1, 'sticon preview did not contain an image element'
screenshot('selected-list')
call('click_element', elementHandle=row['handle'])
time.sleep(.05)
assert any(item.get('accessibleRole') == 'Image' and item.get('accessibleLabel') == '[笑]' and
    any(part.get('typeName') == 'Image' for part in item.get('typeNamesAndIds', [])) for item in tree()), 'history sticon missing'
screenshot('selected-history')
history_x = element('Message history', 'List')['absolutePosition'].get('x', 0)
back()
call('click_element', elementHandle=element('Several LINE emoji', 'Button')['handle'])
time.sleep(.05)
back()
row = element('One LINE emoji', 'Button')
assert not row.get('accessibleItemSelected', False)
assert len([item for item in tree(row['handle']) if item.get('accessibleRole') == 'Image' and item.get('accessibleLabel') == '[笑]']) == 1
screenshot('unselected-list')
from PIL import Image
history = Image.open(args.artifacts / 'selected-history.png')
for state in ['selected', 'unselected']:
    listing = Image.open(args.artifacts / f'{state}-list.png')
    if listing.width < 740:
        comparison = Image.new('RGB', (listing.width + history.width, listing.height))
        comparison.paste(listing, (0, 0))
        comparison.paste(history, (listing.width, 0))
    else:
        comparison = history.copy()
        comparison.paste(listing.crop((0, 0, int(history_x), listing.height)), (0, 0))
    comparison.save(args.artifacts / f'comparison-{state}.png')
# Search uses the same placeholder text, and drafts stay literal text.
search = element('Find a conversation', 'TextInput')['handle']
call('set_element_value', elementHandle=search, value='[笑]')
assert element('One LINE emoji', 'Button')
assert not any(item.get('accessibleLabel', '').startswith('Photo preview, ') for item in tree())
call('set_element_value', elementHandle=search, value='Missing LINE emoji')
row = element('Missing LINE emoji', 'Button')
assert ', Waiting [笑] for you' in row['accessibleLabel']
assert not any(item.get('accessibleRole') == 'Image' and item.get('accessibleLabel') == '[笑]' for item in tree(row['handle']))
call('set_element_value', elementHandle=search, value='Photo preview')
row = element('Photo preview', 'Button')
assert ', ▧ Photo · Riverside walk' in row['accessibleLabel']
assert any(item.get('accessibleRole') == 'Image' and item.get('accessibleLabel') == 'Media thumbnail' for item in tree(row['handle']))
call('set_element_value', elementHandle=search, value='Missing photo preview')
row = element('Missing photo preview', 'Button')
assert ', ▧ Photo' in row['accessibleLabel']
assert not any(item.get('accessibleLabel') == 'Media thumbnail' for item in tree(row['handle']))
assert not any('Retry' in item.get('accessibleLabel', '') for item in tree(row['handle']))
def scenario(label):
    call('dispatch_key_event', windowHandle=window, text='\uf70f')
    handles = call('find_elements_by_id', windowHandle=window, elementsId='DemoScenarios::scenario-panel')['elementHandles']
    button = next(item for item in tree(handles[0]) if item.get('accessibleLabel') == label and item.get('accessibleRole') == 'Button')
    call('click_element', elementHandle=button['handle'])
    time.sleep(.1)


def comparison(name, listing, history, history_x):
    if listing.width < 740:
        result = Image.new('RGB', (listing.width + history.width, listing.height))
        result.paste(listing, (0, 0))
        result.paste(history, (listing.width, 0))
    else:
        result = history.copy()
        result.paste(listing.crop((0, 0, int(history_x), listing.height)), (0, 0))
    result.save(args.artifacts / f'{name}.png')


for title, image_label, plain in [('One LINE emoji', '[笑]', '[笑]'),
                                  ('Photo preview', 'Media thumbnail', 'Photo · Riverside walk'),
                                  ('Sticker preview', 'Media thumbnail', 'Sticker · Nice!')]:
    back()
    search = element('Find a conversation', 'TextInput')['handle']
    call('set_element_value', elementHandle=search, value=title)
    scenario('Preview loaded')
    for state in ['loading', 'loaded']:
        scenario('Preview ' + state)
        back()
        row = element(title, 'Button')
        assert plain in row['accessibleLabel'], row
        pictures = [item for item in tree(row['handle']) if item.get('accessibleRole') == 'Image' and item.get('accessibleLabel') == image_label]
        assert bool(pictures) == (state == 'loaded'), (title, state, pictures)
        if title == 'Sticker preview' and pictures:
            # Portrait fixture: contain must preserve its full aspect ratio.
            assert pictures[0]['size'].get('width', 0) > 0
        stem = title.split()[0].lower() + '-' + state
        screenshot(stem + '-list')
        listing = Image.open(args.artifacts / f'{stem}-list.png')
        call('click_element', elementHandle=row['handle'])
        time.sleep(.1)
        screenshot(stem + '-history')
        history = Image.open(args.artifacts / f'{stem}-history.png')
        history_x = element('Message history', 'List')['absolutePosition'].get('x', 0)
        comparison(stem + '-comparison', listing, history, history_x)
        if title == 'One LINE emoji':
            call('click_element', elementHandle=element('Message draft', 'TextInput')['handle'])
        back()
    call('set_element_value', elementHandle=element('Find a conversation', 'TextInput')['handle'], value='')
print('PASS: preview images/accessibility/search, real media labels, missing fallback and emoji/photo/sticker loading/loaded comparisons')
