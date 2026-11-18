"""Merge a finished port from a Renovate branch and tag the release it makes.

upstream-land.yml calls this when a CI workflow finishes on a renovate/gpui-*
branch. It acts only when the branch's open pull request carries `port-ready`
(set by upstream-port.yml), still has HEAD_SHA as its head, and the latest run of
every CI workflow (CI_WORKFLOWS) on that commit succeeded. It then merges the
pull request as the maintainer and pushes one tag per crate in TAGS whose
version has no tag yet, which starts release.yml.

Environment:
  HEAD_SHA, HEAD_BRANCH   the CI run's commit and branch
  CI_WORKFLOWS            comma-separated workflow names
  MERGE_METHOD            squash or merge
  TAGS                    "crate=tag-prefix,..."; empty for repositories that do not release
  RELEASE_BRANCHES        comma-separated branches whose port must release the first crate in TAGS
"""

import json
import os
import subprocess
import sys
import urllib.error
import urllib.request

REPO = os.environ["GITHUB_REPOSITORY"]
TOKEN = os.environ["MAINTAINER_TOKEN"]
SHA = os.environ["HEAD_SHA"]
BRANCH = os.environ["HEAD_BRANCH"]


def env_list(name):
    return [item.strip() for item in os.environ.get(name, "").split(",") if item.strip()]


def api(method, path, body=None):
    request = urllib.request.Request(
        f"https://api.github.com/{path}",
        method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    with urllib.request.urlopen(request) as response:
        return json.loads(response.read() or "null")


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def ci_failure():
    """Why CI does not allow the merge, or None."""
    names = set(env_list("CI_WORKFLOWS"))
    runs = api("GET", f"repos/{REPO}/actions/runs?head_sha={SHA}&per_page=100")["workflow_runs"]
    latest = {}
    for run in sorted(runs, key=lambda r: r["created_at"]):
        if run["name"] in names:
            latest[run["name"]] = run
    if not latest:
        return "no CI workflow ran"
    for name, run in sorted(latest.items()):
        if run["conclusion"] != "success":
            return f"{name} is {run['conclusion'] or run['status']}"
    return None


def versions(ref):
    """{crate: version} of the workspace at `ref`."""
    tree = f"{os.environ['RUNNER_TEMP']}/land-{ref[:12]}"
    git("worktree", "add", "--detach", tree, ref)
    try:
        metadata = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1"],
            cwd=tree,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    finally:
        git("worktree", "remove", "--force", tree)
    return {p["name"]: p["version"] for p in json.loads(metadata)["packages"]}


def main():
    pulls = api("GET", f"repos/{REPO}/pulls?state=open&head={REPO.split('/')[0]}:{BRANCH}")
    pr = next((p for p in pulls if p["head"]["sha"] == SHA), None)
    if pr is None or "port-ready" not in {label["name"] for label in pr["labels"]}:
        print(f"No open port-ready pull request for {BRANCH} at {SHA}; nothing to do.")
        return
    if reason := ci_failure():
        print(f"#{pr['number']} not merged: {reason}.")
        return

    tags = []
    if entries := [entry.split("=", 1) for entry in env_list("TAGS")]:
        new, old = versions(SHA), versions(f"origin/{pr['base']['ref']}")
        existing = set(git("tag", "--list").split())
        for crate, prefix in entries:
            tag = f"{prefix}{new[crate]}"
            if new[crate] != old.get(crate) and tag not in existing:
                tags.append(tag)
        main_crate, prefix = entries[0]
        if BRANCH in env_list("RELEASE_BRANCHES") and f"{prefix}{new[main_crate]}" not in tags:
            sys.exit(
                f"::error::{main_crate} {new[main_crate]} is not a new release "
                f"(base has {old.get(main_crate)}); the port must bump its version."
            )

    try:
        merged = api(
            "PUT",
            f"repos/{REPO}/pulls/{pr['number']}/merge",
            {
                "merge_method": os.environ.get("MERGE_METHOD", "squash"),
                "sha": SHA,
                "commit_title": f"{pr['title']} (#{pr['number']})",
            },
        )
    except urllib.error.HTTPError as error:
        message = json.loads(error.read() or "{}").get("message", "")
        sys.exit(f"::error::#{pr['number']} not merged ({error.code}): {message}")
    print(f"#{pr['number']} merged as {merged['sha']}.")
    api("DELETE", f"repos/{REPO}/git/refs/heads/{BRANCH}")

    # Pushed with the maintainer's token, so each tag starts release.yml.
    url = f"https://x-access-token:{TOKEN}@github.com/{REPO}.git"
    git("fetch", "-q", "origin")
    for tag in tags:
        git("tag", tag, merged["sha"])
        git("push", "-q", url, f"refs/tags/{tag}")
        print(f"Tagged {tag}.")


if __name__ == "__main__":
    main()
