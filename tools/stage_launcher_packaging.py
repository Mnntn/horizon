#!/usr/bin/env python3
"""Stage repository-specific launcher packaging without modifying upstream templates.

The returned directory contains the existing packaging scripts and their private inputs.
Run linux/appimage.sh or macos/package.sh from that directory as usual.
"""
import argparse
import shutil
from pathlib import Path

UPSTREAM = 'horizon-rust/horizon'
FORK = 'arismoko/horizon'

def stage(source: Path, destination: Path, repository: str):
    if repository not in (UPSTREAM, FORK):
        raise ValueError(f'Unsupported launcher repository: {repository}')
    shutil.copytree(source, destination)
    if repository == UPSTREAM:
        return
    replacements = {
        'horizon-launcher': 'horizon-fork-launcher',
        'horizon-fork-launcher.desktop': 'io.github.arismoko.horizonfork.launcher.desktop',
        'dev.horizon.launcher': 'io.github.arismoko.horizonfork.launcher',
        'Horizon.app': 'Horizon Fork Launcher (arismoko).app',
        '<string>Horizon</string>': '<string>Horizon Fork Launcher (arismoko)</string>',
        'Name=Horizon\n': 'Name=Horizon Fork Launcher (arismoko)\n',
        'StartupWMClass=horizon-fork-launcher': 'StartupWMClass=io.github.arismoko.horizonfork.launcher',
    }
    for name in ('linux/appimage.sh', 'linux/horizon-launcher.desktop', 'macos/package.sh', 'macos/Info.plist'):
        path = destination / name
        text = path.read_text()
        for old, new in replacements.items():
            text = text.replace(old, new)
        if name == 'macos/package.sh':
            # Spaces in the fork's bundle name must remain one shell argument.
            text = text.replace('app=$out/Horizon Fork Launcher (arismoko).app', 'app="$out/Horizon Fork Launcher (arismoko).app"')
            text = text.replace('--keepParent Horizon Fork Launcher (arismoko).app', '--keepParent "Horizon Fork Launcher (arismoko).app"')
            text = text.replace('identity=${MACOS_SIGN_IDENTITY:--}', 'identity=-')
            text = text.replace('"title": "Horizon"', '"title": "Horizon Fork Launcher (arismoko)"')
            text = text.replace('"$out/dmg.json" Horizon "$dmg"', '"$out/dmg.json" "Horizon Fork Launcher (arismoko)" "$dmg"')
        path.write_text(text)
    (destination / 'linux/horizon-launcher.desktop').rename(destination / 'linux/io.github.arismoko.horizonfork.launcher.desktop')
    shutil.copyfile(source / 'icons/fork/launcher.ico', destination / 'icons/horizon.ico')
    shutil.copyfile(source / 'icons/fork/launcher.icns', destination / 'icons/horizon.icns')
    for size in (16, 24, 32, 48, 64, 128, 256, 512):
        shutil.copyfile(source / f'icons/fork/{size}.png', destination / f'icons/hicolor/{size}x{size}/apps/horizon.png')
    shutil.copyfile(source / 'icons/fork/launcher.svg', destination / 'icons/hicolor/scalable/apps/horizon.svg')

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--repository', default=FORK, choices=(UPSTREAM, FORK))
    args = parser.parse_args()
    stage(Path(__file__).resolve().parents[1] / 'packaging', args.destination, args.repository)
