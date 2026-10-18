#!/usr/bin/env python3
"""Exercise S6 with synthetic data in an already running --demo MCP window.

Agents must launch their owned preview with tools/run-ui-debug.py; do not map
a debug window on the user's desktop. Pass that scope's unique --port here.
"""
import argparse
import json
import time
import urllib.request

from mcp_preview import add_preview_arguments, launch_preview

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--port', type=int, default=9457)
add_preview_arguments(parser)
args = parser.parse_args()
launch_preview(args)
serial = 0


def rpc(method, params):
    global serial
    serial += 1
    request = urllib.request.Request(f'http://127.0.0.1:{args.port}/mcp', data=json.dumps({'jsonrpc': '2.0', 'id': serial, 'method': method, 'params': params}).encode(), headers={'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream'})
    with urllib.request.urlopen(request, timeout=10) as response:
        result = json.load(response)
    assert 'error' not in result, result
    return result['result']


def call(name, **arguments):
    result = rpc('tools/call', {'name': name, 'arguments': arguments})
    assert not result.get('isError'), result
    return json.loads(result['content'][0]['text'])


rpc('initialize', {'protocolVersion': '2025-03-26', 'capabilities': {}, 'clientInfo': {'name': 'line-composer-check', 'version': '1'}})
window = call('list_windows')['windowHandles'][0]
root = call('get_window_properties', windowHandle=window)['rootElementHandle']


def tree():
    result = call('get_element_tree', elementHandle=root, maxElements=1000)
    assert not result.get('truncated')
    return result['elements']


def element(label, role=None):
    items = [item for item in tree() if role is None or item.get('accessibleRole') == role]
    return next(item for item in items if item.get('accessibleLabel') == label or (role == 'Button' and item.get('accessibleLabel', '').startswith(label + ', ')))


def click(label, role='Button'):
    call('click_element', elementHandle=element(label, role)['handle'])


def key(text, event_type='PressAndRelease'):
    call('dispatch_key_event', windowHandle=window, text=text, eventType=event_type)


def value():
    return call('get_element_properties', elementHandle=element('Message draft', 'TextInput')['handle']).get('accessibleValue', '')


def set_draft(text):
    call('set_element_value', elementHandle=element('Message draft', 'TextInput')['handle'], value=text)


def demo_scenario(label):
    key('\uf70f')  # F12
    close = element('Close demo scenarios', 'Button')
    position = {'x': close['absolutePosition']['x'] + close['size']['width'] / 2,
                'y': close['absolutePosition']['y'] + close['size']['height'] + 60}
    for delta in (0, 10000, *([-320] * 12)):
        if delta:
            call('dispatch_pointer_scroll', windowHandle=window, position=position, deltaY=delta)
        try:
            action = element(label, 'Button')
            break
        except StopIteration:
            pass
    else:
        raise AssertionError('demo scenario was not reachable: ' + label)
    call('click_element', elementHandle=action['handle'])


def wait_for(check):
    deadline = time.monotonic() + 5
    while not check():
        assert time.monotonic() < deadline, 'demo send did not settle'
        time.sleep(.05)


def message_count(text):
    return sum(item.get('accessibleValue') == text for item in tree()
               if item.get('accessibleRole') == 'TextInput'
               and any(base.get('id', '').endswith('::body') for base in item.get('typeNamesAndIds', [])))


# This marker refuses an authenticated window. Sends use the local demo transport.
# Run once at the default demo size and once with --demo-size 1400x900.
key('\uf70f')  # F12
element('Synthetic interaction scenarios', 'Text')
click('Close demo scenarios')
click('葵')
set_draft('Profile shortcut draft')
window_width = call('get_element_properties', elementHandle=root)['size']['width']
click('Friends')
call('set_element_value', elementHandle=element('Search friends and groups', 'TextInput')['handle'], value='葵')
click('Open profile: 葵')
editors = [item for item in tree() if item.get('accessibleLabel') == 'Message draft']
profile_kind = 'panel' if editors else 'sheet'
assert bool(editors) == (window_width >= 1060), 'covered composer accessibility does not match profile layout'
if editors:
    click('Message draft', 'TextInput')
key('\x11', 'Press')
key('\n')
key('\x11', 'Release')
element('Close details', 'Button')
if editors:
    assert value() == '', 'Ctrl+Enter beside the profile panel did not send'
    wait_for(lambda: message_count('Profile shortcut draft') == 1)
else:
    assert not any(item.get('accessibleLabel') == 'Message history' for item in tree()), 'sheet exposed the covered history'
if editors:
    click('Close details')
else:
    click('Search friends and groups', 'TextInput')
    key('\x1b')
assert not any(item.get('accessibleLabel') == 'Close details' for item in tree()), 'Escape from browser did not close the sheet/profile'
assert value() == ('' if editors else 'Profile shortcut draft'), 'profile Ctrl+Enter changed the wrong draft'
assert element('Search friends and groups', 'TextInput').get('accessibleValue') == '葵', 'Escape cleared the browser query before closing the sheet'
click('Friends')
click('Open profile: 葵')
click('Search friends and groups', 'TextInput')
key('\x11', 'Press')
key('2')
key('\x11', 'Release')
assert not any(item.get('accessibleLabel') == 'Close details' for item in tree()), 'Ctrl+2 from browser left profile open'
assert element('Chats', 'Button').get('accessibleItemSelected') is True
click('Friends')
click('Open profile: 葵')
click('Settings')
assert not any(item.get('accessibleLabel') == 'Close details' for item in tree()), 'Settings left the profile open'
key('\x1b')
click('Friends')
click('Open profile: 葵')
click('Search friends and groups', 'TextInput')
key('\x12', 'Press')
key('\uf701')
key('\x12', 'Release')
assert not any(item.get('accessibleLabel') == 'Close details' for item in tree()), 'Alt+Down left an unrelated profile open'
click('Chats')
click('葵')
set_draft('日本語 🐈\nsecond line\n')
click('Message draft', 'TextInput')
key('\x11', 'Press')
key('\uf729')
key('\x11', 'Release')
key('\x10', 'Press')
key('\uf703')
key('\uf703')
key('\x10', 'Release')
click('Weekend plans')
set_draft('other chat draft')
click('葵')
# Restored selection replaces exactly the first two Japanese characters.
key('X')
assert value() == 'X語 🐈\nsecond line\n', value()
# Clicking the unchanged chat must preserve a new selection too.
key('\x11', 'Press')
key('a')
key('\x11', 'Release')
click('葵')
key('日本語 🐈\nline two\n')
assert value() == '日本語 🐈\nline two\n', value()
before = value()
key('\x1b')
assert value() == before
key('\n')
assert value() == before + '\n'
click('Settings')
preference = element('Enter sends')
assert preference.get('accessibleChecked') in (False, None), preference
call('click_element', elementHandle=preference['handle'])
key('\x1b')
click('Message draft', 'TextInput')
before = value()
key('\n')  # Enter sends uses the same local transport.
assert value() == ''
set_draft(before)
click('Message draft', 'TextInput')
key('\x11', 'Press')
key('\uf72b')  # Restore the caret after the submitted draft was cleared.
key('\x11', 'Release')
key('\x10', 'Press')
key('\n')
key('\x10', 'Release')
assert value() == before + '\n'
click('Settings')
assert element('Enter sends').get('accessibleChecked') is True
call('click_element', elementHandle=element('Enter sends')['handle'])
key('\x1b')
set_draft('Keep this newer draft')
demo_scenario('Rejected send')
element('Sending', 'Image')
time.sleep(1.2)
element('Not sent', 'Image')
click('Edit')
element('Use message text?', 'Text')
assert value() == 'Keep this newer draft'
click('Cancel')
assert value() == 'Keep this newer draft'
click('Edit')
click('Append')
assert value().startswith('Keep this newer draft\n')
click('Edit')
click('Replace')
assert value() == '駅に着いたら連絡してね。See you soon! 🐈'
click('Retry')
wait_for(lambda: not any(item.get('accessibleLabel') == 'Sending' for item in tree()))
element('Sent', 'Image')
set_draft('Preserve while uncertain')
demo_scenario('Uncertain send')
element('Sending', 'Image')
time.sleep(1.2)
element('Unconfirmed', 'Image')
click('Resend')
element('Send again?', 'Text')
assert any('could create a duplicate' in item.get('accessibleLabel', '') for item in tree())
click('Cancel')
assert value() == 'Preserve while uncertain'
click('Resend')
# The confirmation also uses Resend: select its last matching button.
buttons = [item for item in tree() if item.get('accessibleLabel') == 'Resend' and item.get('accessibleRole') == 'Button']
call('click_element', elementHandle=buttons[-1]['handle'])
element('Unconfirmed', 'Image')  # Original remains eligible for its late echo.
assert value() == 'Preserve while uncertain'
assert not any(item.get('accessibleLabel') == 'Sign in again' for item in tree())
# Finish the previous resend before switching chats while its models reconcile.
wait_for(lambda: not any(item.get('accessibleLabel') == 'Sending' for item in tree()))
time.sleep(.05)
# Real editor input, Send, and a one-shot transport failure with confirmed Resend.
click('Notes to self')
set_draft('')
click('Message draft', 'TextInput')
key('Demo composer send 日本語')
click('Send')
assert value() == '', 'Send did not clear the draft'
wait_for(lambda: message_count('Demo composer send 日本語') == 1)
wait_for(lambda: not any(item.get('accessibleLabel') == 'Sending' for item in tree()))
element('Sent', 'Image')
demo_scenario('Fail next send')
click('Message draft', 'TextInput')
key('Resend this demo message')
key('\x11', 'Press')
key('\n')
key('\x11', 'Release')
assert value() == '', 'failed submission did not clear the draft'
assert message_count('Resend this demo message') == 1
element('Sending', 'Image')
wait_for(lambda: any(item.get('accessibleLabel') == 'Unconfirmed' for item in tree()))
set_draft('Draft after failure')
click('Resend')
element('Send again?', 'Text')
click('Cancel')
assert message_count('Resend this demo message') == 1, 'Cancel resubmitted the message'
assert value() == 'Draft after failure'
click('Resend')
buttons = [item for item in tree() if item.get('accessibleLabel') == 'Resend' and item.get('accessibleRole') == 'Button']
call('click_element', elementHandle=buttons[-1]['handle'])
wait_for(lambda: message_count('Resend this demo message') == 2)
wait_for(lambda: not any(item.get('accessibleLabel') == 'Sending' for item in tree()))
element('Sent', 'Image')
element('Unconfirmed', 'Image')
assert value() == 'Draft after failure', 'Resend changed the newer draft'
demo_scenario('Reject next send')
set_draft('')
click('Message draft', 'TextInput')
key('Retry this rejected message')
click('Send')
assert value() == ''
wait_for(lambda: any(item.get('accessibleLabel') == 'Not sent' for item in tree()))
assert message_count('Retry this rejected message') == 1
set_draft('Draft after failure')
click('Retry')
wait_for(lambda: not any(item.get('accessibleLabel') in ('Sending', 'Not sent') for item in tree()))
assert message_count('Retry this rejected message') == 1, 'Retry duplicated the rejected row'
assert value() == 'Draft after failure'
element('Sent', 'Image')
demo_scenario('Offline')
assert not element('Send', 'Button').get('accessibleEnabled', False)
click('Message draft', 'TextInput')
key('\x11', 'Press')
key('\n')
key('\x11', 'Release')
assert value() == 'Draft after failure', 'offline Ctrl+Enter consumed the draft'
print('PASS: profile ' + profile_kind + ' Ctrl+Enter/Escape/destination/Alt+Down, real demo Send/Ctrl+Enter/history append/draft clearing/failure/Resend/Cancel, S6 MCP per-chat Japanese/emoji caret-selection, unchanged-chat guard, multiline, Escape, persisted-in-session Enter sends, Shift+Enter, distinct rejection/uncertainty marks, Retry, Edit Replace/Append/Cancel, duplicate confirmation and draft preservation; physical IME remains P—')
