#!/usr/bin/env python3
"""Install the built app, launcher, and original icon for this user."""
import fetch_emoji_font
import argparse
import os
from pathlib import Path
import shutil
import shlex
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
build_info = json.loads(subprocess.check_output([str(binary), "--build-info"]))
if build_info["demo"] or build_info["mcp_debug"]:
    parser.error("refusing to install a demo or UI-debug build; rebuild without features")
if args.destdir:
    bin_dir = args.destdir.resolve() / "bin"
    data_dir = args.destdir.resolve() / "share"
else:
    bin_dir = Path.home() / ".local/bin"
    data_dir = Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
launcher = data_dir / "applications" / f"{APP_ID}.desktop"
icon = data_dir / "icons/hicolor/scalable/apps" / f"{APP_ID}.svg"
service = data_dir / "dbus-1/services" / f"{APP_ID}.service"
for directory in (bin_dir, launcher.parent, icon.parent, service.parent):
    directory.mkdir(parents=True, exist_ok=True)
destination = bin_dir / "line-desktop"
# Atomic replacement also permits updating an executable that is running.
temporary = destination.with_suffix(".new")
shutil.copyfile(binary, temporary)
temporary.chmod(0o755)
temporary.replace(destination)
try:
    font_files = list(fetch_emoji_font.verified_files())
except (OSError, ValueError) as error:
    print(f"Warning: emoji font unavailable: {error}", file=sys.stderr)
else:
    font_dir = data_dir / "line-desktop/fonts"
    font_dir.mkdir(parents=True, exist_ok=True)
    for source in font_files:
        shutil.copyfile(source, font_dir / source.name)
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
    return '"' + value.replace("\\", "\\\\").replace("%", "%%") + '"'

arguments = [destination]
command = "/usr/bin/env -- " + " ".join(quoted(v) for v in arguments)
# D-Bus service Exec uses shell-like quoting without desktop field-code expansion.
service_command = " ".join(shlex.quote(str(v)) for v in arguments)
service.write_text(f"[D-BUS Service]\nName={APP_ID}\nExec={service_command}\n")
service.chmod(0o644)
icon_value = str(icon.resolve()).replace("\\", "\\\\")
launcher.write_text(f"""[Desktop Entry]
Type=Application
Name=閃
Comment=Native messaging client
Exec={command}
Icon={icon_value}
Terminal=false
Categories=Network;InstantMessaging;
StartupWMClass={APP_ID}
SingleMainWindow=true
StartupNotify=true
DBusActivatable=true
Keywords=chat;messaging;
""")
launcher.chmod(0o644)
print(f"Installed app: {destination}")
print(f"Installed launcher: {launcher}")
print(f"Installed icon: {icon}")
print(f"Installed D-Bus service: {service}")

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
