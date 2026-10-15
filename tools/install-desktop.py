#!/usr/bin/env python3
"""Install the built app, launcher, and original icon for this user."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import json
import sys

APP_ID = "org.lineclient.Desktop"
ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--destdir", type=Path, help="stage bin/ and share/ under this directory instead of installing")
args = parser.parse_args()
binary = ROOT / "target/release/line-desktop"
if not binary.is_file():
    parser.error("build first: cargo build --release -p line-desktop")
if json.loads(subprocess.check_output([str(binary), "--build-info"]))["mcp_debug"]:
    parser.error("refusing to install a UI-debug build; rebuild without --features ui-debug")
if args.destdir:
    bin_dir = args.destdir.resolve() / "bin"
    data_dir = args.destdir.resolve() / "share"
else:
    bin_dir = Path.home() / ".local/bin"
    data_dir = Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
launcher = data_dir / "applications" / f"{APP_ID}.desktop"
icon = data_dir / "icons/hicolor/scalable/apps" / f"{APP_ID}.svg"
for directory in (bin_dir, launcher.parent, icon.parent):
    directory.mkdir(parents=True, exist_ok=True)
destination = bin_dir / "line-desktop"
# Atomic replacement also permits updating an executable that is running.
temporary = destination.with_suffix(".new")
shutil.copyfile(binary, temporary)
temporary.chmod(0o755)
temporary.replace(destination)
shutil.copyfile(ROOT / "app/assets/icon.svg", icon)
# Icon loaders watch the theme root, not individual application directories.
# Updating an existing icon otherwise leaves Plasma's cached misses intact.
os.utime(icon.parents[2], None)

def quoted(value):
    value = str(value)
    if "\n" in value or "\r" in value:
        raise ValueError("desktop paths must not contain newlines")
    for char in ("\\", '"', "`", "$"):
        value = value.replace(char, "\\" + char)
    return '"' + value.replace("%", "%%") + '"'

command = " ".join(quoted(v) for v in (
    destination, "--state", ROOT / "artifacts/auth/session.json",
    "--wasm", ROOT / "artifacts/line-3.7.2/ltsm.wasm",
))
icon_value = str(icon.resolve()).replace("\\", "\\\\")
launcher.write_text(f"""[Desktop Entry]
Type=Application
Name=LINE Native
Comment=Native messaging client
Exec={command}
Icon={icon_value}
Terminal=false
Categories=Network;InstantMessaging;
StartupWMClass={APP_ID}
""")
launcher.chmod(0o644)
print(f"Installed app: {destination}")
print(f"Installed launcher: {launcher}")
print(f"Installed icon: {icon}")

# Staging must not change the current desktop session's caches.
if not args.destdir:
    for candidates, arguments in (
        (("update-desktop-database",), [str(launcher.parent)]),
        (("kbuildsycoca6", "kbuildsycoca5"), ["--noincremental"]),
    ):
        executable = next((path for name in candidates if (path := shutil.which(name))), None)
        if executable:
            try:
                subprocess.run([executable, *arguments], check=True, timeout=30)
            except (OSError, subprocess.SubprocessError) as error:
                print(f"Warning: desktop cache refresh failed: {error}", file=sys.stderr)
