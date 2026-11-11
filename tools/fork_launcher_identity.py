#!/usr/bin/env python3
"""Mechanical extraction of launcher identity literals. Run once from repository root."""
from pathlib import Path

REPLACEMENTS = {
    'account.rs': {'"dev.playhorizon.launcher"': 'crate::identity::KEYCHAIN_SERVICE'},
    'data_location.rs': {'"horizon-launcher-location"': 'crate::identity::LOCATION_FOLDER'},
    'mac_update.rs': {'"horizon-launcher"': 'crate::identity::EXECUTABLE'},
    'installer.rs': {
        '"Horizon Launcher"': 'crate::identity::DISPLAY_NAME',
        '"horizon-launcher.exe"': 'crate::identity::WINDOWS_EXECUTABLE',
        '"Horizon"': 'crate::identity::PUBLISHER',
        '"https://playhorizon.dev"': 'crate::identity::ABOUT_URL',
        'r"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\HorizonLauncher"': 'crate::identity::UNINSTALL_KEY',
        'r"Software\\Horizon\\Launcher"': 'crate::identity::INSTALL_KEY',
        '"horizon-launcher-uninstall-"': 'crate::identity::UNINSTALL_PREFIX',
        '"horizon-launcher-setup"': 'crate::identity::SETUP_PREFIX',
        '"IMAGENAME eq horizon-launcher.exe"': 'crate::identity::PROCESS_FILTER',
        '"horizon-launcher.exe."': 'crate::identity::WINDOWS_EXECUTABLE_PREFIX',
        '"horizon-launcher"': 'crate::identity::EXECUTABLE',
        'r"Software\\Horizon"': 'crate::identity::REGISTRY_PARENT',
    },
}
for name, replacements in REPLACEMENTS.items():
    path = Path('launcher/src') / name
    text = path.read_text()
    # Test fixtures are independent inputs, not implementation literals.
    boundary = text.find('#[cfg(test)]')
    if boundary < 0:
        boundary = len(text)
    code, tests = text[:boundary], text[boundary:]
    for old, new in replacements.items():
        assert old in code, (name, old)
        code = code.replace(old, new)
    path.write_text(code + tests)
