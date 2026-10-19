#!/usr/bin/env python3
"""Run an offline demo on a private headless backend; stop its scope on exit."""
import argparse
import contextlib
import json
import os
from pathlib import Path
import resource
import secrets
import shutil
import signal
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
UI_TEMP = ROOT / "target" / "u"
UI_TEMP.mkdir(parents=True, exist_ok=True)


@contextlib.contextmanager
def launch_demo(binary, port, arguments=(), log=None):
    binary = Path(binary).resolve()
    if not 1024 <= port <= 65535:
        raise ValueError('use an unprivileged MCP port')
    if not shutil.which('dbus-run-session'):
        raise RuntimeError('dbus-run-session is required for the private headless session')
    if any(value in ('--check-live', '--state', '--wasm') for value in arguments):
        raise ValueError('this launcher accepts offline demo arguments only')
    arguments = ['--demo', '--demo-size', '980x720', *arguments, '--debug-background']
    environment = os.environ.copy()
    for key in ('WAYLAND_DISPLAY', 'WAYLAND_SOCKET', 'DISPLAY', 'XAUTHORITY',
                'XDG_ACTIVATION_TOKEN', 'DESKTOP_STARTUP_ID', 'DBUS_SESSION_BUS_ADDRESS',
                'WINIT_UNIX_BACKEND', 'SLINT_BACKEND', 'XMODIFIERS', 'GTK_IM_MODULE', 'QT_IM_MODULE'):
        environment.pop(key, None)
    token = secrets.token_hex(24)
    environment.update(LINE_UI_INSTANCE=token, LINE_DESKTOP_CREDENTIALS='plaintext-file',
                       SLINT_SCALE_FACTOR='1', SLINT_BACKEND='headless-software',
                       SLINT_MCP_PORT=str(port))
    with tempfile.TemporaryDirectory(dir=UI_TEMP, prefix='d-') as directory:
        environment['LINE_DEMO_DIRECTORY'] = directory
        environment['TMPDIR'] = directory
        for name in ('XDG_RUNTIME_DIR', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME',
                     'XDG_CACHE_HOME', 'XDG_STATE_HOME'):
            path = Path(directory) / name
            path.mkdir(mode=0o700)
            environment[name] = str(path)
        info = json.loads(subprocess.check_output([str(binary), '--demo', '--build-info'], text=True, env=environment))
        if not info.get('mcp_debug'):
            raise ValueError('build the ui-debug binary first')
        config = Path(directory) / 'session.conf'
        config.write_text(f'''<busconfig>
  <type>session</type>
  <listen>unix:tmpdir={directory}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
''')
        process = subprocess.Popen(
            ['dbus-run-session', '--config-file', str(config), '--', str(binary), *arguments],
            env=environment, stdout=log, stderr=log, start_new_session=True,
            preexec_fn=lambda: resource.setrlimit(resource.RLIMIT_CORE, (0, 0)))
        process.demo_token = token
        process.demo_directory = Path(directory)
        previous_handlers = {}
        def terminate(_signum, _frame):
            raise KeyboardInterrupt
        for signum in (signal.SIGTERM, signal.SIGHUP):
            previous_handlers[signum] = signal.signal(signum, terminate)
        try:
            yield process
        finally:
            for signum, handler in previous_handlers.items():
                signal.signal(signum, handler)
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', default='release')
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--port', type=int, required=True)
    parser.add_argument('--log', type=Path)
    parser.add_argument('arguments', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    binary = args.binary or ROOT / 'target' / ('debug' if args.profile == 'dev' else args.profile) / 'line-desktop'
    arguments = args.arguments[1:] if args.arguments[:1] == ['--'] else args.arguments
    def terminate(_signum, _frame):
        raise KeyboardInterrupt
    for signum in (signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, terminate)
    with args.log.open('w') if args.log else contextlib.nullcontext() as log:
        try:
            with launch_demo(binary, args.port, arguments, log) as process:
                print(f'Headless demo scope PID: {process.pid}; MCP port: {args.port}', flush=True)
                return process.wait()
        except KeyboardInterrupt:
            return 0


if __name__ == '__main__':
    raise SystemExit(main())
