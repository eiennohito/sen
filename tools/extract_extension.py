#!/usr/bin/env python3
"""Extract just the WASM and provenance from an official LINE Chrome CRX3."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import struct
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('crx', type=Path)
parser.add_argument('--output', type=Path, default=Path('artifacts'))
args = parser.parse_args()
blob = args.crx.read_bytes()
if blob[:4] != b'Cr24' or struct.unpack_from('<I', blob, 4)[0] != 3:
    parser.error('expected a CRX3 file')
header_size = struct.unpack_from('<I', blob, 8)[0]
with zipfile.ZipFile(io.BytesIO(blob[12 + header_size:])) as archive:
    manifest = json.loads(archive.read('manifest.json'))
    wasm = archive.read('static/js/ltsm.wasm')
if wasm[:4] != b'\x00asm':
    parser.error('archive did not contain a WASM module')
dest = args.output / ('line-' + manifest['version'])
dest.mkdir(parents=True, exist_ok=True)
(dest / 'ltsm.wasm').write_bytes(wasm)
metadata = {
    'extension_id': 'ophjlpahpchlmihnnnihgmmeilfjmjjc',
    'version': manifest['version'],
    'wasm_sha256': hashlib.sha256(wasm).hexdigest(),
    'crx_sha256': hashlib.sha256(blob).hexdigest(),
    'wasm_bytes': len(wasm),
    'source': 'https://clients2.google.com/service/update2/crx',
    'note': 'Extracted from the downloaded archive; CRX signature is not independently verified.',
}
(dest / 'provenance.json').write_text(json.dumps(metadata, indent=2) + '\n')
print(json.dumps(metadata, indent=2))
print('Extracted:', dest / 'ltsm.wasm')
