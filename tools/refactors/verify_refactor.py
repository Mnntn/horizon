"""Replay committed refactors on their historical input and compare their output."""
import io
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile


def verify(script: Path, application_subject: str) -> None:
    root = script.resolve().parents[2]
    def git(*args: str) -> bytes:
        return subprocess.check_output(['git', '-C', str(root), *args])
    # The newest commit with that subject in HEAD's history: the refactors have landed on main, so no branch range.
    commits = git('log', '--format=%H%x09%s', 'HEAD').decode().splitlines()
    commit = next(line.split('\t')[0] for line in commits if line.split('\t', 1)[1] == application_subject)
    with tempfile.TemporaryDirectory(prefix='verify-refactor-') as name:
        snapshot = Path(name)
        with tarfile.open(fileobj=io.BytesIO(git('archive', commit + '^'))) as archive:
            # The archive comes from this repository's own committed tree.
            archive.extractall(snapshot)
        subprocess.run([sys.executable, str(snapshot / script.relative_to(root))], check=True)
        paths = git('diff', '--name-only', commit + '^', commit).decode().splitlines()
        for path in paths:
            result = subprocess.run(['git', '-C', str(root), 'show', f'{commit}:{path}'], capture_output=True)
            actual = snapshot / path
            if result.returncode:
                assert not actual.exists(), f'{path}: expected removal'
            else:
                assert actual.read_bytes() == result.stdout, f'{path}: output differs'
    print(f'{script.name}: verified mechanical output at {commit}')
