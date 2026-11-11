#!/usr/bin/env python3
"""One-shot, text-preserving relocation for the fork's gate/data workflow split.

Run on the pre-split tree. Policy and semantic test changes belong in later commits.
"""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
source = ROOT / ".github/workflows/dyna-ci.yml"
destination = ROOT / ".github/workflows/main-data.yml"
if destination.exists():
    raise SystemExit("main-data.yml already exists; run this migration on the pre-split tree")
text = source.read_text()
start = text.index("  # The #[ignore]d library tests")
end = text.index("  # The launcher is its own workspace", start)
ignored = text[start:end]
text = text[:start] + text[end:]
start = text.index("  # The fork's Dyna runners have the owner's prepared vanilla data")
manual = text[start:]
text = text[:start]
# Extract the existing shared routing behavior without changing its shell body.
start = text.index("        run: |\n", text.index("      - id: runner\n"))
end = text.index("      - id: hosted\n", start)
block = text[start:end]
body = block.removeprefix("        run: |\n")
assert all(not line or line.startswith("          ") for line in body.splitlines())
routing = "#!/usr/bin/env bash\nset -eo pipefail\n" + "\n".join(line[10:] for line in body.splitlines()) + "\n"
(ROOT / "tools/ci/dyna_ci_route.sh").write_text(routing)
text = text[:start] + '        run: bash tools/ci/dyna_ci_route.sh\n' + text[end:]
source.write_text(text)
destination.write_text("""name: main-data
on:
  workflow_dispatch:
permissions:
  contents: read
env:
  HORIZON_TELEMETRY_URL: "off"
  CARGO_INCREMENTAL: 0
jobs:
""" + ignored + manual)
