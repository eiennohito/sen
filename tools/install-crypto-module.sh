#!/bin/sh
# Install the supported crypto module from the newest installed Chrome LINE extension.
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec python3 - "$script_dir/../src/lib.rs" <<'PY'
import hashlib
import os
from pathlib import Path
import re
import sys
import tempfile

try:
    home = Path(os.environ['HOME'])
    if not home.is_absolute():
        raise ValueError('HOME must be an absolute path')
    extension = 'ophjlpahpchlmihnnnihgmmeilfjmjjc'
    versions = []
    for directory in (home / '.config/google-chrome').glob(f'*/Extensions/{extension}/*'):
        if directory.is_dir() and re.fullmatch(r'\d+(?:\.\d+)*(?:_\d+)?', directory.name):
            version, _, revision = directory.name.partition('_')
            versions.append((tuple(map(int, version.split('.'))), int(revision or 0), directory))
    if not versions:
        raise ValueError('no installed Chrome LINE extension found')
    source = max(versions)[2] / 'static/js/ltsm.wasm'
    module = source.read_bytes()
    # Use the exact SHA-256 pin checked by the app's crypto-module prerequisite.
    expected = re.search(r'pub const HASH: &str = "([0-9a-f]{64})";', Path(sys.argv[1]).read_text())[1]
    if hashlib.sha256(module).hexdigest() != expected:
        raise ValueError(f'unsupported crypto module: {source}')
    data = Path(os.environ.get('XDG_DATA_HOME', ''))
    if not data.is_absolute():
        data = home / '.local/share'
    destination = data / 'line-desktop/ltsm.wasm'
    destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=destination.parent, delete=False) as output:
            temporary = Path(output.name)
            os.fchmod(output.fileno(), 0o600)
            output.write(module)
            output.flush()
            os.fsync(output.fileno())
        temporary.replace(destination)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    print(f'Installed verified crypto module: {destination}')
except (OSError, ValueError, KeyError) as error:
    sys.exit(f'install-crypto-module: {error}')
PY
