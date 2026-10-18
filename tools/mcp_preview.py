"""Optional owned preview for the MCP checks; existing --port connections still work."""
import atexit
import json
from pathlib import Path
import subprocess
import sys
import tempfile


def add_preview_arguments(parser):
    parser.add_argument('--launch', action='store_true', help='Launch and stop an owned offline preview')
    parser.add_argument('--profile', default=None, help='Cargo profile passed to run-ui-debug.py with --launch')
    parser.add_argument('--demo-size', default=None, help='Preview size with --launch')


def launch_preview(args):
    if not args.launch:
        if args.profile is not None or args.demo_size is not None:
            raise SystemExit("--profile and --demo-size require --launch")
        return
    args.profile = args.profile or "release"
    args.demo_size = args.demo_size or "980x720"
    directory = tempfile.TemporaryDirectory(prefix='line-mcp-check-')
    log = (Path(directory.name) / 'preview.log').open('w+')
    command = [sys.executable, str(Path(__file__).with_name('run-ui-debug.py')),
        '--profile', args.profile, '--port', str(args.port),
        '--pid-file', str(Path(directory.name) / 'scope.json'), '--log', log.name,
        '--', '--demo', '--demo-size', args.demo_size]
    if getattr(args, 'signed_out', False):
        command += ['--demo-signed-out']
    if getattr(args, 'missing_module', False):
        command += ['--snapshot-case', 'missing-module']
    if hasattr(args, 'text_size'):
        command += ['--demo-text-size', str(args.text_size), '--demo-color-scheme', args.color_scheme]
    if hasattr(args, 'color_scheme'):
        command += ['--demo-dark' if args.color_scheme == 'dark' else '--demo-light']
    process = subprocess.Popen(command, stdout=subprocess.PIPE, text=True)

    def stop():
        if process.poll() is None:
            process.terminate()
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            raise RuntimeError('preview wrapper did not stop its owned scope')
        process.stdout.close()
        log.close()
        directory.cleanup()

    atexit.register(stop)
    line = process.stdout.readline()
    try:
        scope = json.loads(line)
        assert scope['port'] == args.port
    except (ValueError, KeyError, AssertionError):
        log.seek(0)
        raise RuntimeError(f'owned preview failed: {log.read()}') from None
    print(f'Preview: {line.strip()}', flush=True)
