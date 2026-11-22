#!/usr/bin/env python3
"""Mirror the canonical Majordomus plan (.ai/repo/project) to GitHub milestones and issues.

The .ai records are the source of truth; GitHub is a projection. Every projected milestone and
issue carries a marker naming its canonical record, so a run finds what it created before and
edits it instead of creating a duplicate. Nothing is ever deleted on GitHub; a record that is
DONE or CANCELLED closes its issue, anything else reopens it.

    plan_github.py --milestone records-and-status-tell-the-truth --dry-run
    plan_github.py --milestone records-and-status-tell-the-truth
    plan_github.py --all

Repository-agnostic: the GitHub repository comes from `repository:` in .ai/repo/project/project.yaml
(or --repo), the data from `majordomus plan`, the writes from `gh api`.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

MARKER = "plan-github"
MARKER_RE = re.compile(r"<!-- plan-github: (\S+) -->")
CLOSED_STATES = {"DONE", "CANCELLED"}
BODY_LIMIT = 60000
STATUS_LABELS = {
    "READY": ("plan/ready", "0e8a16", "All dependencies satisfied; can be started"),
    "BLOCKED": ("plan/blocked", "b60205", "Waits on another issue"),
    "ACTIVE": ("plan/active", "fbca04", "Execution has started"),
    "VERIFY": ("plan/verify", "5319e7", "Implemented; evidence pending"),
    "DONE": ("plan/done", "cfd3d7", "Completed with evidence"),
    "CANCELLED": ("plan/cancelled", "ffffff", "Will not be done"),
}
PRIORITY_COLORS = {"p0": "b60205", "p1": "d93f0b", "p2": "fbca04", "p3": "c5def5"}


def run(cmd, stdin=None):
    out = subprocess.run(cmd, input=stdin, capture_output=True, text=True)
    if out.returncode != 0:
        raise SystemExit(f"{' '.join(cmd[:4])}... failed ({out.returncode}): {out.stderr.strip()}")
    return out.stdout


def majordomus(*args):
    return run(["majordomus", *args])


def repo_root():
    return Path(run(["git", "rev-parse", "--show-toplevel"]).strip())


def repository_from_project(root):
    project = root / ".ai/repo/project/project.yaml"
    for line in project.read_text().splitlines():
        m = re.match(r"^repository:\s*['\"]?([\w.-]+/[\w.-]+)", line)
        if m:
            return m.group(1)
    raise SystemExit(f"no repository: in {project}; pass --repo")


def default_branch(root):
    project = (root / ".ai/repo/project/project.yaml").read_text()
    m = re.search(r"^default_branch:\s*['\"]?([\w./-]+)", project, re.M)
    return m.group(1) if m else "main"


def marker(path):
    return f"<!-- {MARKER}: {path} -->"


def milestone_number(roadmap, milestone_id):
    """M<n> as the plan shows it: the 1-based position in derived order."""
    ordered = sorted(roadmap, key=lambda m: (m.get("order", 0), m["id"]))
    for n, m in enumerate(ordered, 1):
        if m["id"] == milestone_id:
            return n
    raise SystemExit(f"unknown milestone {milestone_id}")


def milestone_title(n, title):
    title = f"M{n}: {title}"
    return title if len(title) <= 120 else title[:117].rstrip() + "..."


def issue_title(issue_id, title):
    title = f"{issue_id}: {title}"
    return title if len(title) <= 250 else title[:247].rstrip() + "..."


def linkify(body, repo, branch):
    """Turn `.ai/...` canonical paths in backticks into links to the default branch."""
    def sub(m):
        path = m.group(1)
        return f"[`{path}`](https://github.com/{repo}/blob/{branch}/{path})"
    return re.sub(r"`(\.ai/[^`\s]+\.ya?ml)`", sub, body)


def issue_body(issue_id, path, repo, branch):
    body = linkify(majordomus("plan", "body", issue_id), repo, branch)
    footer = (
        "\n\n---\n"
        f"Projected from the canonical record [`{path}`](https://github.com/{repo}/blob/{branch}/{path}) "
        "by `tools/plan-github`. Edit the record, not this issue: the next sync overwrites the body. "
        "Status, labels and open/closed state are derived by `majordomus plan`.\n"
        f"{marker(path)}\n"
    )
    if len(body) + len(footer) > BODY_LIMIT:
        body = body[: BODY_LIMIT - len(footer) - 40] + "\n\n_(truncated; see the record)_"
    return body + footer


def milestone_description(milestone_id, path):
    show = majordomus("plan", "show", milestone_id)
    m = re.search(r"## Outcome\s+(.+?)(\n## |\Z)", show, re.S)
    outcome = (m.group(1).strip() if m else "").replace("\n", " ")
    text = f"{outcome}\n\nCanonical: {path}\n{marker(path)}"
    return text if len(text) <= 1000 else outcome[: 1000 - len(path) - 80] + f"...\n\nCanonical: {path}\n{marker(path)}"


class GitHub:
    def __init__(self, repo, dry_run):
        self.repo, self.dry_run = repo, dry_run

    def api(self, method, path, payload=None, paginate=False):
        cmd = ["gh", "api", "-X", method, f"repos/{self.repo}" + (f"/{path}" if path else "")]
        if paginate:
            cmd.insert(2, "--paginate")
        if payload is not None:
            cmd += ["--input", "-"]
        out = run(cmd, json.dumps(payload) if payload is not None else None)
        if paginate:  # --paginate concatenates JSON arrays as ][
            out = "[" + out.strip().lstrip("[").rstrip("]").replace("]\n[", ",").replace("][", ",") + "]"
        return json.loads(out) if out.strip() else None

    def write(self, method, path, payload, what):
        print(f"{'DRY ' if self.dry_run else ''}{method:5} {what}")
        return None if self.dry_run else self.api(method, path, payload)


def ensure_labels(gh, wanted):
    existing = {l["name"]: l for l in gh.api("GET", "labels?per_page=100", paginate=True)}
    for name, (color, desc) in sorted(wanted.items()):
        cur = existing.get(name)
        if cur is None:
            gh.write("POST", "labels", {"name": name, "color": color, "description": desc}, f"label {name}")
        elif cur["color"].lower() != color or (cur.get("description") or "") != desc:
            gh.write("PATCH", f"labels/{name.replace('/', '%2F')}", {"color": color, "description": desc}, f"label {name}")


def sync_milestones(gh, root, roadmap, ids):
    existing = {}
    for m in gh.api("GET", "milestones?state=all&per_page=100", paginate=True):
        found = MARKER_RE.search(m.get("description") or "")
        if found:
            existing[found.group(1)] = m
    numbers = {}
    for mid in ids:
        rec = next(m for m in roadmap if m["id"] == mid)
        path = f".ai/repo/project/milestones/{mid}.yaml"
        payload = {
            "title": milestone_title(milestone_number(roadmap, mid), rec["title"]),
            "description": milestone_description(mid, path),
            "state": "closed" if rec["status"] in CLOSED_STATES else "open",
        }
        cur = existing.get(path)
        if cur is None:
            made = gh.write("POST", "milestones", payload, f"milestone {payload['title']}")
            numbers[mid] = made["number"] if made else None
        else:
            numbers[mid] = cur["number"]
            if any(cur.get(k) != v for k, v in payload.items()):
                gh.write("PATCH", f"milestones/{cur['number']}", payload, f"milestone {payload['title']}")
    return numbers


def sync_issues(gh, repo, branch, issues, milestone_numbers):
    existing = {}
    for i in gh.api("GET", "issues?state=all&labels=plan&per_page=100", paginate=True):
        found = MARKER_RE.search(i.get("body") or "")
        if found and "pull_request" not in i:
            existing[found.group(1)] = i
    for rec in issues:
        path = f".ai/repo/project/issues/{rec['id']}.yaml"
        labels = ["plan", f"priority/{rec['priority']}", STATUS_LABELS[rec["status"]][0]]
        payload = {
            "title": issue_title(rec["id"], rec["title"]),
            "body": issue_body(rec["id"], path, repo, branch),
            "labels": labels,
            "milestone": milestone_numbers.get(rec["milestone"]),
        }
        state = "closed" if rec["status"] in CLOSED_STATES else "open"
        cur = existing.get(path)
        if cur is None:
            made = gh.write("POST", "issues", payload, f"issue {payload['title'][:90]}")
            if made and state == "closed":
                gh.write("PATCH", f"issues/{made['number']}", {"state": "closed", "state_reason": "completed"}, f"close #{made['number']}")
            continue
        changed = {k: v for k, v in payload.items() if k != "labels" and (cur.get(k) if k != "milestone" else (cur.get("milestone") or {}).get("number")) != v}
        if sorted(l["name"] for l in cur.get("labels", [])) != sorted(labels):
            changed["labels"] = labels
        if cur["state"] != state:
            changed["state"] = state
        if changed:
            gh.write("PATCH", f"issues/{cur['number']}", changed, f"#{cur['number']} {sorted(changed)}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--milestone", action="append", default=[], help="milestone id to project (repeatable)")
    ap.add_argument("--all", action="store_true", help="project every milestone and issue")
    ap.add_argument("--repo", help="owner/name (default: repository: in project.yaml)")
    ap.add_argument("--dry-run", action="store_true", help="print the writes without making them")
    args = ap.parse_args()
    if not args.all and not args.milestone:
        ap.error("name --milestone <id> or --all")

    root = repo_root()
    repo = args.repo or repository_from_project(root)
    branch = default_branch(root)
    roadmap = json.loads(majordomus("plan", "roadmap", "--json"))["milestones"]
    issues = json.loads(majordomus("plan", "list", "--json"))["issues"]
    ids = [m["id"] for m in roadmap] if args.all else args.milestone
    unknown = set(ids) - {m["id"] for m in roadmap}
    if unknown:
        raise SystemExit(f"unknown milestone(s): {', '.join(sorted(unknown))}")
    selected = [i for i in issues if i["milestone"] in ids]

    gh = GitHub(repo, args.dry_run)
    info = gh.api("GET", "")
    if not info.get("has_issues"):
        raise SystemExit(f"{repo} has issues disabled; enable them first (gh repo edit {repo} --enable-issues)")
    labels = {
        "plan": ("1d76db", "Projected from the canonical .ai plan record; edit the record"),
        "proposal": ("c2e0c6", "Not in the plan yet; a worker turns it into a .ai record"),
    }
    labels.update({n: (c, d) for n, c, d in STATUS_LABELS.values()})
    labels.update({f"priority/{p}": (c, f"Plan priority {p}") for p, c in PRIORITY_COLORS.items()})
    ensure_labels(gh, labels)
    numbers = sync_milestones(gh, root, roadmap, ids)
    sync_issues(gh, repo, branch, selected, numbers)
    print(f"{repo}: {len(ids)} milestone(s), {len(selected)} issue(s) {'checked (dry run)' if args.dry_run else 'in sync'}")


if __name__ == "__main__":
    sys.exit(main())
