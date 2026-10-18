#!/usr/bin/env python3
"""Run an offline MCP demo on a private virtual KWin output, without mapping a host window.

Normal app launches are unaffected. The owned private bus/compositor exits with
its demo. Use the reported app PID and a unique MCP port for exclusive control.
"""
import argparse
import json
import os
from pathlib import Path
import resource
import secrets
import shutil
import signal
import socket as sockets
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--profile', default='release', help='Cargo profile containing the ui-debug binary')
parser.add_argument('--binary', type=Path)
parser.add_argument('--port', type=int, required=True)
parser.add_argument('--pid-file', type=Path)
parser.add_argument('--log', type=Path)
parser.add_argument('arguments', nargs=argparse.REMAINDER)
args = parser.parse_args()
binary = (args.binary or ROOT / 'target' / ('debug' if args.profile == 'dev' else args.profile) / 'line-desktop').resolve()
if not 1024 <= args.port <= 65535:
    parser.error('use an unprivileged unique MCP port')
try:
    probe = sockets.socket()
    # Match the server: a stopped fresh instance may leave TCP TIME_WAIT sockets.
    probe.setsockopt(sockets.SOL_SOCKET, sockets.SO_REUSEADDR, 1)
    probe.bind(('127.0.0.1', args.port))
    probe.close()
except OSError:
    parser.error('MCP port is already occupied; choose a unique port')
try:
    info = json.loads(subprocess.check_output([str(binary), '--build-info'], text=True))
except (OSError, subprocess.SubprocessError, ValueError):
    parser.error('build the ui-debug binary first')
if not info.get('mcp_debug'):
    parser.error(f'requires cargo build --profile {args.profile} --offline -p line-desktop --features ui-debug')
for tool in ('dbus-run-session', 'kwin_wayland'):
    if not shutil.which(tool):
        parser.error(f'{tool} is required; refusing to fall back to a host window')
arguments = args.arguments[1:] if args.arguments[:1] == ['--'] else args.arguments
if any(value in ('--check-live', '--state', '--wasm') for value in arguments):
    parser.error('this automation launcher accepts offline demo arguments only')
if not any(value.startswith('--demo') for value in arguments):
    arguments = ['--demo', *arguments]
arguments = [*arguments, '--debug-background']
socket = f'line-desktop-automation-{os.getpid()}-{secrets.token_hex(4)}'
pid_file = (args.pid_file or Path(f'/tmp/{socket}.json')).resolve()
if pid_file.exists():
    parser.error('pid-file already exists; use a new owned path')
environment = os.environ.copy()
for key in ('WAYLAND_DISPLAY', 'DISPLAY', 'XDG_ACTIVATION_TOKEN', 'DESKTOP_STARTUP_ID', 'DBUS_SESSION_BUS_ADDRESS'):
    environment.pop(key, None)
# KDE_DEBUG disables KCrash: a crashing private KWin must not restart itself
# or open DrKonqi on the host desktop.
environment.update(QT_QPA_PLATFORM='offscreen', QT_QUICK_BACKEND='software', KWIN_COMPOSE='Q', KDE_DEBUG='1')
# The private bus has no keyring: credentials, if any, use the development
# plaintext store and can never prompt on, or write to, the host keyring.
environment['LINE_DESKTOP_CREDENTIALS'] = 'plaintext-file'
def owns_port(pid, port):
    try:
        inodes = {os.readlink(fd)[8:-1] for fd in Path(f'/proc/{pid}/fd').iterdir() if os.readlink(fd).startswith('socket:[')}
        for table in ('tcp', 'tcp6'):
            for line in Path(f'/proc/{pid}/net/{table}').read_text().splitlines()[1:]:
                fields = line.split()
                if fields[3] == '0A' and int(fields[1].split(':')[1], 16) == port and fields[9] in inodes:
                    return True
    except (OSError, IndexError):
        pass
    return False
log = args.log.open('w') if args.log else None
with tempfile.TemporaryDirectory(prefix='line-ui-automation-') as directory:
    session = Path(directory) / 'session'
    session.write_text('#!/usr/bin/python3\nimport os,json\nfrom pathlib import Path\n' +
        f'os.environ["WAYLAND_DISPLAY"]={socket!r}\n' +
        f'os.environ["LINE_DESKTOP_AUTOMATION_SOCKET"]={socket!r}\n' +
        'os.environ["WINIT_UNIX_BACKEND"]="wayland"\n' +
        f'os.environ["SLINT_MCP_PORT"]={str(args.port)!r}\n' +
        f'Path({str(pid_file)!r}).write_text(json.dumps({{"app_pid":os.getpid(),"socket":{socket!r},"port":{args.port}}}))\n' +
        f'os.execv({str(binary)!r},{[str(binary), *arguments]!r})\n')
    session.chmod(0o700)
    command = ['dbus-run-session', '--', 'kwin_wayland', '--virtual', '--socket', socket,
        '--width', '1280', '--height', '900', '--scale', '1', '--no-lockscreen',
        '--no-global-shortcuts', '--no-kactivities', '--exit-with-session', str(session)]
    def terminate_owned_scope(_signum, _frame):
        raise KeyboardInterrupt
    for signum in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, terminate_owned_scope)
    # The private KWin (and occasionally the app) segfaults; keep those crashes
    # out of the OS reporting. With a piped core_pattern (systemd-coredump ->
    # DrKonqi) a limit of 0 still reports the crash; the kernel skips the pipe
    # only when RLIMIT_CORE is exactly 1. Inherited by everything in the scope.
    process = subprocess.Popen(command, env=environment, stdout=log, stderr=log, start_new_session=True,
        preexec_fn=lambda: resource.setrlimit(resource.RLIMIT_CORE, (1, 1)))
    try:
        for _ in range(200):
            if pid_file.exists():
                try:
                    ownership = json.loads(pid_file.read_text())
                except (ValueError, OSError):
                    time.sleep(.05)
                    continue
                if ownership.get('socket') != socket or ownership.get('port') != args.port:
                    raise SystemExit('refusing a PID record from another automation scope')
                if not owns_port(ownership.get('app_pid'), args.port):
                    time.sleep(.05)
                    continue
                ownership.update(scope_pid=process.pid, pid_file=str(pid_file))
                print(json.dumps(ownership), flush=True)
                break
            if process.poll() is not None:
                raise SystemExit('virtual compositor failed; no host-window fallback was attempted')
            time.sleep(.05)
        else:
            raise SystemExit('virtual compositor did not start the demo')
        raise SystemExit(process.wait())
    except KeyboardInterrupt:
        pass
    finally:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        if process.poll() is None:
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        if log:
            log.close()
