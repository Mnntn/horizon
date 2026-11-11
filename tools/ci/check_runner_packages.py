#!/usr/bin/env python3
"""Fail when the Dyna CI runner image's apt packages differ from tools/ci/runner-packages.txt, in either direction.

The image is built outside this repository, so whoever changes it runs this against its Containerfile:
    python3 tools/ci/check_runner_packages.py path/to/Containerfile
"""
import re
import sys
from pathlib import Path

LIST = Path(__file__).with_name("runner-packages.txt")


def image_packages(containerfile):
    text = re.sub(r"\\\n", " ", containerfile)
    packages = []
    for command in re.split(r"&&|;|\n", text):
        words = command.split()
        if "apt-get" in words and "install" in words:
            packages += [word for word in words[words.index("install") + 1:] if not word.startswith("-")]
    return packages


def main(listing=LIST):
    image = image_packages(Path(sys.argv[1]).read_text())
    if not image:
        sys.exit("no apt-get install found in the Containerfile")
    listed = [line for line in listing.read_text().splitlines() if line and not line.startswith("#")]
    missing = [package for package in image if package not in listed]
    extra = [package for package in listed if package not in image]
    if missing or extra:
        sys.exit(f"runner-packages.txt lacks: {' '.join(missing) or '-'}; has beyond the image: {' '.join(extra) or '-'}")
    print(f"runner-packages.txt matches the image's {len(image)} apt packages")


if __name__ == "__main__":
    main()
