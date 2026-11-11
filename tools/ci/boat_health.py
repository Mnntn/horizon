#!/usr/bin/env python3
"""Choose a hosted runner using repository-local Actions history, never boat secrets."""
from datetime import datetime, timezone
import io
import json
import os
import re
import subprocess
import sys
import zipfile

ROUTING_WORKFLOW = ".github/workflows/boat-recovery.yml"
ROUTING_ARTIFACT = "boat-routing"
ROUTING_MEMBER = "boat-routing.json"
LOGIN = re.compile(r"[A-Za-z0-9](?:[A-Za-z0-9]|-(?=[A-Za-z0-9])){0,38}")


def user_step_started(job):
    return any(step.get("started_at") for step in job.get("steps", [])
               if step.get("name") not in ("Set up job", "Complete job"))


def genuine_failure(jobs):
    return any(job.get("conclusion") in ("failure", "timed_out") and user_step_started(job) for job in jobs)


def recovery_reason(jobs, now):
    """An outage must not hide any genuine failure in the original run."""
    if genuine_failure(jobs):
        return ""
    return health_reason([job for job in jobs if job["name"] in ("lint", "verify")], now)


def previous_attempt_reason(jobs, now):
    """Retain an outage across cancellation; rerun APIs cannot override labels."""
    reason = recovery_reason(jobs, now)
    if reason:
        return reason
    if genuine_failure(jobs):
        return ""
    for job in jobs:
        if (job["name"] in ("lint", "verify") and "boat-large" in job.get("labels", [])
                and job.get("conclusion") == "cancelled" and not user_step_started(job)):
            created = datetime.fromisoformat(job["created_at"].replace("Z", "+00:00"))
            completed = datetime.fromisoformat(job["completed_at"].replace("Z", "+00:00"))
            if (completed - created).total_seconds() > 600:
                return f"previous attempt boat job {job['id']} cancelled after waiting more than 10 minutes"
    return ""


def health_reason(jobs, now):
    """Return a runner outage reason, not an ordinary build/test failure."""
    for job in jobs:
        if "boat-large" not in job.get("labels", []):
            continue
        if job["status"] == "queued":
            created = datetime.fromisoformat(job["created_at"].replace("Z", "+00:00"))
            if (now - created).total_seconds() > 600:
                return f"boat job {job['id']} queued for more than 10 minutes"
        if job.get("conclusion") == "failure" and not user_step_started(job):
            return f"boat job {job['id']} failed before its first step"
    return ""


def select_label(enabled, users, author, actor, jobs, now):
    if enabled != "on" or author not in users or actor not in users:
        return "ubuntu-latest", "boat is disabled or the author/actor is not listed"
    reason = health_reason(jobs, now)
    return ("ubuntu-latest", reason) if reason else ("boat-large", "recent boat jobs show no runner outage")


def api(path):
    result = subprocess.run(["gh", "api", path], text=True, capture_output=True, check=True)
    return json.loads(result.stdout)


def download(path):
    return subprocess.run(["gh", "api", path], capture_output=True, check=True).stdout


def routing(document, run_id):
    """The switch and users of a published routing document, or None when it is not exactly the bounded schema."""
    if not isinstance(document, dict) or set(document) != {"schema", "enabled", "users", "run_id"}:
        return None
    users = document["users"]
    if (document["schema"] != 1 or document["enabled"] not in ("on", "off") or document["run_id"] != str(run_id)
            or not isinstance(users, list) or len(users) > 100
            or not all(isinstance(user, str) and LOGIN.fullmatch(user) for user in users)):
        return None
    return document["enabled"], users


def publish(enabled, users, run_id):
    """Mirror this trusted run's variables for pull requests from forks, whose runs see no variables."""
    document = {"schema": 1, "enabled": "on" if enabled == "on" else "off",
                "users": [user.strip() for user in users.split(",") if user.strip()], "run_id": str(run_id)}
    text = json.dumps(document)
    if routing(json.loads(text), run_id) is None or len(text) > 8192:
        raise SystemExit("BOAT_CI_USERS is not a list of at most 100 GitHub logins")
    return text


def published_routing(repo, now):
    """Read the newest routing artifact of the default branch's own boat-recovery runs; fail closed with a reason."""
    branch = api(f"repos/{repo}")["default_branch"]
    runs = api(f"repos/{repo}/actions/workflows/boat-recovery.yml/runs?branch={branch}&per_page=5")["workflow_runs"]
    for run in runs:
        if (run["path"] != ROUTING_WORKFLOW or run["head_branch"] != branch
                or run["repository"]["full_name"] != repo or run["head_repository"]["full_name"] != repo
                or run["event"] not in ("schedule", "workflow_run")):
            continue
        artifacts = api(f"repos/{repo}/actions/runs/{run['id']}/artifacts?name={ROUTING_ARTIFACT}")["artifacts"]
        if not artifacts:
            continue  # Started but not published yet, or expired.
        if len(artifacts) != 1:
            return None, "the boat routing run has more than one routing artifact"
        artifact = artifacts[0]
        origin = artifact["workflow_run"]
        created = datetime.fromisoformat(artifact["created_at"].replace("Z", "+00:00"))
        if (artifact["name"] != ROUTING_ARTIFACT or artifact["expired"] or artifact["size_in_bytes"] > 16384
                or origin["id"] != run["id"] or origin["repository_id"] != origin["head_repository_id"]):
            return None, "the boat routing artifact is not this repository's bounded artifact"
        if (now - created).total_seconds() > 1200:
            return None, "the boat routing artifact is older than 20 minutes"
        try:
            with zipfile.ZipFile(io.BytesIO(download(f"repos/{repo}/actions/artifacts/{artifact['id']}/zip"))) as zipped:
                entries = zipped.infolist()
                if len(entries) != 1 or entries[0].filename != ROUTING_MEMBER or entries[0].file_size > 8192:
                    return None, "the boat routing artifact does not hold exactly one bounded document"
                document = json.loads(zipped.read(entries[0]))
        except Exception:
            # Codec and parser errors of any kind mean the document is unusable.
            return None, "the boat routing artifact is malformed"
        result = routing(document, run["id"])
        return (result, "") if result else (None, "the boat routing document does not match its schema")
    return None, "no recent boat routing artifact"


def main():
    repo = os.environ["GITHUB_REPOSITORY"]
    jobs = []
    reason = ""
    enabled = os.environ.get("BOAT_CI", "")
    users = [user.strip() for user in os.environ.get("BOAT_USERS", "").split(",") if user.strip()]
    author, actor = os.environ["AUTHOR"], os.environ["ACTOR"]
    if os.environ.get("BOAT_FALLBACK") == "true":
        enabled = "off"
    elif not enabled:
        # A pull request from a fork sees no variables; the default branch's recovery runs publish them.
        try:
            published, unavailable = published_routing(repo, datetime.now(timezone.utc))
        except (subprocess.CalledProcessError, json.JSONDecodeError, KeyError, TypeError, ValueError) as failure:
            published, unavailable = None, f"boat routing is unavailable ({type(failure).__name__})"
        if published:
            enabled, users = published
        else:
            print(f"boat routing: {unavailable}")
    if enabled == "on" and author in users and actor in users:
        try:
            # Bound the read cost: five recent runs, at most 100 jobs per run.
            for run in api(f"repos/{repo}/actions/runs?per_page=5")["workflow_runs"]:
                jobs.extend(api(f"repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100")["jobs"])
        except (subprocess.CalledProcessError, json.JSONDecodeError, KeyError) as failure:
            reason = f"boat health history is unavailable ({type(failure).__name__})"
    label, decision = select_label(enabled, users, author, actor, jobs, datetime.now(timezone.utc))
    attempt = int(os.environ.get("GITHUB_RUN_ATTEMPT", "1"))
    prior = ""
    if attempt > 1:
        try:
            previous = api(f"repos/{repo}/actions/runs/{os.environ['GITHUB_RUN_ID']}/attempts/{attempt - 1}/jobs?per_page=100")["jobs"]
            prior = previous_attempt_reason(previous, datetime.now(timezone.utc))
        except (subprocess.CalledProcessError, json.JSONDecodeError, KeyError) as failure:
            prior = f"previous attempt history is unavailable ({type(failure).__name__})"
    forced = bool(reason or prior or (label == "ubuntu-latest" and health_reason(jobs, datetime.now(timezone.utc))))
    if reason or prior:
        label, decision = "ubuntu-latest", prior or reason
    print(f"hosted runner: {label}: {decision}")
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        output.write(f"label={label}\nforced_github={str(forced).lower()}\n")
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
        summary.write(f"### Hosted runner selection\n\n{label}: {decision}.\n")


if __name__ == "__main__":
    if sys.argv[1:] == ["publish"]:
        print(publish(os.environ.get("BOAT_CI", ""), os.environ.get("BOAT_USERS", ""), os.environ["GITHUB_RUN_ID"]))
    else:
        main()
