#!/usr/bin/env python3
"""Validate and reconcile LOOM's public five-year roadmap with GitHub.

The tracked manifest contains only public product metadata. Issue identity is
preserved by a stable hidden marker, so reruns update in place instead of
creating duplicates.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from dataclasses import dataclass, field
from datetime import date, timedelta
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_MANIFEST = ROOT / "roadmap" / "roadmap.json"
MARKER_RE = re.compile(r"<!-- loom-roadmap:v\d+ id=(\d{4}) -->")
RETIRED_MARKER_RE = re.compile(r"<!-- loom-roadmap-retired:v\d+ id=(\d{4})\s+merged-into=(\d{4}) -->")
FORGE_MARKER_RE = re.compile(r"<!-- forge-task:v1 id=(\d{4})(?:\s+[^>]*)? -->")
MARKER_PREFIX = "<!-- loom-roadmap:v2 id="
MANAGED_LABEL_PREFIXES = ("type:", "priority:", "horizon:", "area:", "phase:", "status:")
API_VERSION = "2022-11-28"
WRITE_DELAY_SECONDS = 1.1


BASE_LABELS: dict[str, tuple[str, str]] = {
    "type:epic": ("5319e7", "Quarter or phase-level outcome and gate"),
    "type:bug": ("d73a4a", "Reproducible defect or retrieval failure"),
    "type:feature": ("1d76db", "Product or engineering capability"),
    "type:research": ("8250df", "Research, evaluation, or product validation"),
    "type:security": ("b60205", "Security, privacy, or assurance work"),
    "priority:P0": ("b60205", "Current critical path"),
    "priority:P1": ("d93f0b", "Next validated horizon"),
    "priority:P2": ("fbca04", "Planned after the exact-recovery wedge"),
    "priority:P3": ("cfd3d7", "Long-horizon option gated by evidence"),
    "horizon:now": ("0e8a16", "0–6 month execution horizon"),
    "horizon:next": ("1d76db", "6–24 month execution horizon"),
    "horizon:later": ("6f42c1", "Long-horizon option; not a shipped promise"),
    "status:retired": ("ededed", "Planning item consolidated before implementation"),
    "area:product": ("d4c5f9", "Product definition, adoption, and validation"),
    "area:repository": ("ededed", "Repository governance and maintenance"),
    "area:core": ("0e8a16", "Canonical Rust core and domain model"),
    "area:ingestion": ("006b75", "Capture, extraction, and indexing"),
    "area:retrieval": ("0052cc", "Search, ranking, and query behavior"),
    "area:evidence": ("fbca04", "Evidence anchors, source opening, and provenance"),
    "area:evaluation": ("c2e0c6", "Fixtures, metrics, studies, and quality gates"),
    "area:desktop": ("bfdadc", "Tauri and macOS desktop experience"),
    "area:platform": ("d876e3", "Build, release, and platform integration"),
    "area:performance": ("f9d0c4", "Latency, resource, and scale budgets"),
    "area:privacy": ("7f1d1d", "Local-first data boundaries and user controls"),
    "area:security": ("d73a4a", "Threat modeling and security controls"),
    "area:connector": ("c5def5", "Browser and professional source connectors"),
    "area:provenance": ("fef2c0", "Artifact lineage, versions, and relationships"),
    "area:portability": ("bfd4f2", "Export, backup, restore, and migration"),
    "area:accessibility": ("e6e6fa", "Accessible keyboard and evidence workflows"),
    "area:api": ("b4a7d6", "Local APIs and extension contracts"),
    "area:ecosystem": ("cfd3d7", "SDKs, extensions, and community integration"),
    "area:sync": ("9f4f96", "Optional end-to-end encrypted continuity"),
}


@dataclass
class Snapshot:
    labels: list[dict[str, Any]]
    milestones: list[dict[str, Any]]
    issues: list[dict[str, Any]]
    issue_by_task: dict[str, dict[str, Any]]
    subissues: dict[str, set[int]] = field(default_factory=dict)
    blocked_by: dict[str, set[int]] = field(default_factory=dict)


def load_manifest(path: Path) -> dict[str, Any]:
    try:
        data = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot load manifest {path}: {error}") from error
    validate_manifest(data)
    return data


def validate_manifest(data: dict[str, Any]) -> None:
    if data.get("schema_version") != 1:
        raise ValueError("roadmap schema_version must be 1")
    program = data.get("program")
    milestones = data.get("milestones")
    issues = data.get("issues")
    retired = data.get("retired_issues")
    if not isinstance(program, dict) or not isinstance(milestones, list) or not isinstance(issues, list) or not isinstance(retired, list):
        raise ValueError("manifest requires program, milestones, issues, and retired_issues")
    if len(milestones) != 20:
        raise ValueError(f"expected exactly 20 quarterly milestones, found {len(milestones)}")
    if len(issues) < 100:
        raise ValueError(f"expected at least 100 roadmap issues, found {len(issues)}")
    if program.get("issue_count") != len(issues) or program.get("retired_issue_count") != len(retired) or program.get("quarter_count") != len(milestones):
        raise ValueError("program counts do not match manifest contents")

    milestone_keys: dict[str, int] = {}
    previous_due: date | None = None
    for index, milestone in enumerate(milestones, start=1):
        expected_key = f"Q{index:02d}"
        if milestone.get("key") != expected_key:
            raise ValueError(f"milestone {index} must use key {expected_key}")
        key = milestone["key"]
        if key in milestone_keys:
            raise ValueError(f"duplicate milestone key {key}")
        milestone_keys[key] = index
        start = date.fromisoformat(milestone["starts_on"])
        due = date.fromisoformat(milestone["due_on"])
        if due < start:
            raise ValueError(f"milestone {key} ends before it starts")
        if previous_due is not None and start != previous_due + timedelta(days=1):
            raise ValueError(f"milestone {key} is not contiguous with its predecessor")
        previous_due = due
        for field_name in ("title", "focus", "exit_criteria"):
            if not str(milestone.get(field_name, "")).strip():
                raise ValueError(f"milestone {key} is missing {field_name}")
    if milestones[0]["starts_on"] != program.get("starts_on") or milestones[-1]["due_on"] != program.get("ends_on"):
        raise ValueError("quarter boundaries must match the five-year program boundaries")

    ids: set[str] = set()
    by_id: dict[str, dict[str, Any]] = {}
    forbidden = ("gpt-", ".forge/", "/users/", "agent:", "model:", "routing metadata")
    serialized = json.dumps(data).lower()
    for phrase in forbidden:
        if phrase in serialized:
            raise ValueError(f"public manifest contains forbidden private metadata: {phrase}")
    allowed_status = {"backlog", "ready", "in-progress", "review", "blocked", "done"}
    for issue in issues:
        task_id = str(issue.get("id", ""))
        if not re.fullmatch(r"\d{4}", task_id):
            raise ValueError(f"invalid roadmap id {task_id!r}")
        if task_id in ids:
            raise ValueError(f"duplicate roadmap id {task_id}")
        ids.add(task_id)
        by_id[task_id] = issue
        if issue.get("status") not in allowed_status:
            raise ValueError(f"issue {task_id} has invalid status")
        if issue.get("quarter") not in milestone_keys:
            raise ValueError(f"issue {task_id} has no valid quarter")
        if not str(issue.get("phase", "")).strip() or not str(issue.get("workstream", "")).strip():
            raise ValueError(f"issue {task_id} is missing phase or workstream")
        outcome = str(issue.get("outcome", "")).strip()
        if not outcome or "produces a verifiable" in outcome.lower():
            raise ValueError(f"issue {task_id} lacks a concrete outcome")
        criteria = issue.get("acceptance_criteria")
        if not isinstance(criteria, list) or not 2 <= len(criteria) <= 4 or any(not str(item).strip() for item in criteria):
            raise ValueError(f"issue {task_id} must have 2–4 acceptance criteria")
        labels = issue.get("labels")
        if not isinstance(labels, list) or len(labels) != len(set(labels)):
            raise ValueError(f"issue {task_id} has invalid or duplicate labels")
        if sum(label.startswith("type:") for label in labels) != 1:
            raise ValueError(f"issue {task_id} must have exactly one type label")
        if sum(label.startswith("priority:") for label in labels) != 1:
            raise ValueError(f"issue {task_id} must have exactly one priority label")
        if sum(label.startswith("horizon:") for label in labels) != 1:
            raise ValueError(f"issue {task_id} must have exactly one horizon label")
        if not any(label.startswith("area:") for label in labels):
            raise ValueError(f"issue {task_id} must have at least one area label")
        if sum(label.startswith("phase:") for label in labels) != 1:
            raise ValueError(f"issue {task_id} must have exactly one phase label")
        qnum = milestone_keys[issue["quarter"]]
        if qnum <= 8:
            near_term = " ".join([outcome, *map(str, criteria)]).lower()
            if any(token in near_term for token in ("tbd", "to be determined", "eventually", "some tests")):
                raise ValueError(f"near-term issue {task_id} is not implementation-ready")

    if program.get("phase_count") != len({issue["phase"] for issue in issues}):
        raise ValueError("program phase_count does not match issue phases")
    if len({issue["phase"] for issue in issues}) != 13:
        raise ValueError("the five-year program must preserve exactly 13 product phases")
    if any(not any(issue["quarter"] == key for issue in issues) for key in milestone_keys):
        raise ValueError("every quarter must contain at least one issue")

    dependency_count = 0
    for task_id, issue in by_id.items():
        parent = issue.get("parent")
        if parent is not None:
            if parent not in by_id or parent == task_id:
                raise ValueError(f"issue {task_id} has invalid parent {parent}")
            if by_id[parent].get("parent") is not None or "type:epic" not in by_id[parent]["labels"]:
                raise ValueError(f"issue {task_id} parent {parent} is not an epic")
            if milestone_keys[by_id[parent]["quarter"]] > milestone_keys[issue["quarter"]]:
                raise ValueError(f"issue {task_id} is scheduled before parent {parent}")
        dependencies = issue.get("depends_on")
        if not isinstance(dependencies, list) or len(dependencies) != len(set(dependencies)):
            raise ValueError(f"issue {task_id} has invalid or duplicate prerequisites")
        dependency_count += len(dependencies)
        for blocker in dependencies:
            if blocker not in by_id or blocker == task_id:
                raise ValueError(f"issue {task_id} has invalid prerequisite {blocker}")
            if milestone_keys[by_id[blocker]["quarter"]] > milestone_keys[issue["quarter"]]:
                raise ValueError(f"issue {task_id} depends on later-quarter issue {blocker}")

    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(task_id: str) -> None:
        if task_id in visiting:
            raise ValueError(f"dependency cycle reaches {task_id}")
        if task_id in visited:
            return
        visiting.add(task_id)
        for blocker in by_id[task_id]["depends_on"]:
            visit(blocker)
        visiting.remove(task_id)
        visited.add(task_id)

    for task_id in by_id:
        visit(task_id)
    if len(visited) != len(issues):
        raise ValueError("dependency graph did not cover every issue")
    if sum(issue.get("parent") is not None for issue in issues) != len(issues) - 13:
        raise ValueError("expected 13 phase epics and every other issue to have one parent")
    if dependency_count < len(issues):
        raise ValueError("dependency graph is unexpectedly sparse")
    retired_ids: set[str] = set()
    for item in retired:
        task_id = str(item.get("id", ""))
        merged_into = str(item.get("merged_into", ""))
        if not re.fullmatch(r"\d{4}", task_id) or task_id in retired_ids or task_id in by_id:
            raise ValueError(f"invalid or duplicate retired roadmap id {task_id}")
        if merged_into not in by_id or not str(item.get("reason", "")).strip():
            raise ValueError(f"retired roadmap id {task_id} has no valid merge target or reason")
        retired_ids.add(task_id)


def label_specs(manifest: dict[str, Any]) -> dict[str, tuple[str, str]]:
    specs = dict(BASE_LABELS)
    for issue in manifest["issues"]:
        for label in issue["labels"]:
            if label.startswith("phase:") and label not in specs:
                specs[label] = ("b4a7d6", f"Product phase {issue['phase']}")
    desired = {label for issue in manifest["issues"] for label in issue["labels"]}
    missing = desired - set(specs)
    if missing:
        raise ValueError(f"labels have no public specification: {sorted(missing)}")
    return {name: specs[name] for name in sorted(desired | {"type:bug", "status:retired"})}


def issue_marker(task_id: str) -> str:
    return f"{MARKER_PREFIX}{task_id} -->"


def retired_marker(task_id: str, merged_into: str) -> str:
    return f"<!-- loom-roadmap-retired:v2 id={task_id} merged-into={merged_into} -->"


def retired_body(item: dict[str, Any]) -> str:
    return f"""{retired_marker(item['id'], item['merged_into'])}

Roadmap ID: `{item['id']}`<br>
Merged into: `{item['merged_into']}`

## Consolidation

{item['reason']}

This planning item was consolidated before implementation. No implementation, test, study, review,
or release evidence is claimed here. The retained roadmap item carries the combined acceptance
criteria and prerequisites.
"""


def public_body(issue: dict[str, Any], milestone: dict[str, Any]) -> str:
    parent = f"Roadmap parent: `{issue['parent']}`" if issue.get("parent") else "Roadmap phase epic."
    blockers = issue["depends_on"]
    prerequisite_text = "\n".join(f"- Blocked by roadmap `{task_id}`" for task_id in blockers) if blockers else "No blocking roadmap issues."
    criteria = "\n".join(f"- [ ] {criterion}" for criterion in issue["acceptance_criteria"])
    return f"""{issue_marker(issue['id'])}

Roadmap ID: `{issue['id']}`<br>
Quarter: **{issue['quarter']}** — {milestone['title']}<br>
Phase: **{issue['phase']}**<br>
Workstream: **{issue['workstream']}**<br>
{parent}

## Outcome

{issue['outcome']}

## Acceptance criteria

{criteria}

## Prerequisites

{prerequisite_text}

## Closure evidence

{issue['closure_evidence']}

## Product boundary

{issue['product_boundary']}
"""


def run_gh(method: str, path: str, payload: dict[str, Any] | None = None, *, paginate: bool = False) -> Any:
    command = [
        "gh", "api", "--method", method, path,
        "--header", "Accept: application/vnd.github+json",
        "--header", f"X-GitHub-Api-Version: {API_VERSION}",
    ]
    stdin = None
    if paginate:
        if method != "GET" or payload is not None:
            raise ValueError("pagination only supports payload-free GET requests")
        command.extend(["--paginate", "--slurp"])
    if payload is not None:
        command.extend(["--input", "-"])
        stdin = json.dumps(payload)
    for attempt in range(10):
        result = subprocess.run(command, input=stdin, text=True, capture_output=True, check=False)
        if result.returncode == 0:
            parsed = json.loads(result.stdout) if result.stdout.strip() else None
            if paginate:
                if not isinstance(parsed, list) or any(not isinstance(page, list) for page in parsed):
                    raise RuntimeError(f"gh api {method} {path} returned an invalid pagination envelope")
                return [item for page in parsed for item in page]
            return parsed
        lowered = result.stderr.lower()
        retryable = "secondary rate" in lowered or "http 403" in lowered or "http 429" in lowered
        if not retryable or attempt == 9:
            raise RuntimeError(f"gh api {method} {path} failed: {result.stderr.strip()}")
        delay = min(10 * (attempt + 1), 60)
        print(f"GitHub rate limited {method} {path}; retrying in {delay}s", file=sys.stderr, flush=True)
        time.sleep(delay)
    raise RuntimeError(f"gh api {method} {path} exhausted retries")


def index_managed_issues(issues: list[dict[str, Any]], known_ids: set[str]) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for issue in issues:
        body = issue.get("body") or ""
        active = MARKER_RE.search(body)
        retired = RETIRED_MARKER_RE.search(body)
        forge = FORGE_MARKER_RE.search(body)
        found_ids = {match.group(1) for match in (active, retired, forge) if match}
        if not found_ids:
            continue
        if len(found_ids) != 1:
            raise ValueError(f"issue #{issue['number']} contains conflicting roadmap markers {sorted(found_ids)}")
        task_id = found_ids.pop()
        if task_id not in known_ids:
            raise ValueError(f"issue #{issue['number']} has unknown LOOM roadmap marker {task_id}")
        if task_id in result:
            raise ValueError(f"duplicate LOOM roadmap marker {task_id} on issues #{result[task_id]['number']} and #{issue['number']}")
        result[task_id] = issue
    return result


def load_snapshot(repository: str, manifest: dict[str, Any], *, relationships: bool) -> Snapshot:
    labels = run_gh("GET", f"repos/{repository}/labels?per_page=100", paginate=True)
    milestones = run_gh("GET", f"repos/{repository}/milestones?state=all&per_page=100", paginate=True)
    issues = run_gh("GET", f"repos/{repository}/issues?state=all&per_page=100", paginate=True)
    known_ids = {issue["id"] for issue in manifest["issues"]} | {issue["id"] for issue in manifest["retired_issues"]}
    issue_by_task = index_managed_issues(issues, known_ids)
    snapshot = Snapshot(labels, milestones, issues, issue_by_task)
    if not relationships:
        return snapshot
    for issue in manifest["issues"]:
        live = issue_by_task.get(issue["id"])
        if live is None:
            continue
        if issue.get("parent") is None:
            children = run_gh("GET", f"repos/{repository}/issues/{live['number']}/sub_issues?per_page=100", paginate=True)
            snapshot.subissues[issue["id"]] = {child["id"] for child in children}
        blockers = run_gh("GET", f"repos/{repository}/issues/{live['number']}/dependencies/blocked_by?per_page=100", paginate=True)
        snapshot.blocked_by[issue["id"]] = {blocker["id"] for blocker in blockers}
    return snapshot


def labels_on(issue: dict[str, Any]) -> set[str]:
    return {label["name"] if isinstance(label, dict) else str(label) for label in issue.get("labels", [])}


def managed_labels_with_preserved_external(live: dict[str, Any], desired: list[str]) -> list[str]:
    external = {name for name in labels_on(live) if not name.startswith(MANAGED_LABEL_PREFIXES)}
    return sorted(external | set(desired))


def milestone_resolution(manifest: dict[str, Any], snapshot: Snapshot) -> tuple[dict[str, dict[str, Any]], list[dict[str, Any]], list[dict[str, Any]]]:
    by_title = {item["title"]: item for item in snapshot.milestones}
    resolved: dict[str, dict[str, Any]] = {}
    creates: list[dict[str, Any]] = []
    updates: list[dict[str, Any]] = []
    for desired in manifest["milestones"]:
        live = by_title.get(desired["title"])
        if live is None and desired.get("legacy_title"):
            live = by_title.get(desired["legacy_title"])
        if live is None:
            creates.append(desired)
            # A placeholder lets a dry run still report stale bodies, labels,
            # titles, and state for issues whose milestone will be created first.
            resolved[desired["key"]] = {"number": None}
            continue
        resolved[desired["key"]] = live
        live_due = (live.get("due_on") or "")[:10]
        wanted = {
            "title": desired["title"],
            "description": f"{desired['focus']} Exit gate: {desired['exit_criteria']}",
            "due_on": f"{desired['due_on']}T23:59:59Z",
            "state": "open",
        }
        if live.get("title") != wanted["title"] or (live.get("description") or "") != wanted["description"] or live_due != desired["due_on"] or live.get("state") != "open":
            updates.append({"number": live["number"], "key": desired["key"], "payload": wanted})
    return resolved, creates, updates


def build_plan(manifest: dict[str, Any], snapshot: Snapshot, *, include_relationships: bool) -> dict[str, Any]:
    specs = label_specs(manifest)
    live_labels = {item["name"]: item for item in snapshot.labels}
    create_labels = []
    update_labels = []
    for name, (color, description) in specs.items():
        live = live_labels.get(name)
        payload = {"name": name, "color": color, "description": description}
        if live is None:
            create_labels.append(payload)
        elif live.get("color", "").lower() != color.lower() or (live.get("description") or "") != description:
            update_labels.append({"current_name": name, "payload": payload})

    resolved_milestones, create_milestones, update_milestones = milestone_resolution(manifest, snapshot)
    desired_milestones = {item["key"]: item for item in manifest["milestones"]}
    create_issues = []
    update_issues = []
    unexpected_closed = []
    for desired in manifest["issues"]:
        live = snapshot.issue_by_task.get(desired["id"])
        milestone = desired_milestones[desired["quarter"]]
        resolved = resolved_milestones.get(desired["quarter"])
        if live is None:
            create_issues.append(desired["id"])
            continue
        if resolved is None:
            continue
        wanted_labels = managed_labels_with_preserved_external(live, desired["labels"])
        wanted_body = public_body(desired, milestone)
        patch: dict[str, Any] = {}
        if live.get("title") != desired["title"]:
            patch["title"] = desired["title"]
        if (live.get("body") or "") != wanted_body:
            patch["body"] = wanted_body
        if labels_on(live) != set(wanted_labels):
            patch["labels"] = wanted_labels
        if resolved["number"] is not None and (live.get("milestone") or {}).get("number") != resolved["number"]:
            patch["milestone"] = resolved["number"]
        if desired["status"] == "done" and live.get("state") != "closed":
            patch["state"] = "closed"
            patch["state_reason"] = "completed"
        elif desired["status"] != "done" and live.get("state") == "closed":
            unexpected_closed.append(desired["id"])
        if patch:
            update_issues.append({"id": desired["id"], "number": live["number"], "payload": patch})

    retire_issues = []
    for retired in manifest["retired_issues"]:
        live = snapshot.issue_by_task.get(retired["id"])
        if live is None:
            continue
        payload: dict[str, Any] = {}
        title = f"Consolidated roadmap item {retired['id']} into {retired['merged_into']}"
        body = retired_body(retired)
        wanted_labels = managed_labels_with_preserved_external(live, ["status:retired"])
        if live.get("title") != title:
            payload["title"] = title
        if (live.get("body") or "") != body:
            payload["body"] = body
        if labels_on(live) != set(wanted_labels):
            payload["labels"] = wanted_labels
        if live.get("milestone") is not None:
            payload["milestone"] = None
        if live.get("state") != "closed" or live.get("state_reason") != "not_planned":
            payload["state"] = "closed"
            payload["state_reason"] = "not_planned"
        if payload:
            retire_issues.append({"id": retired["id"], "number": live["number"], "payload": payload})

    add_parents: list[list[str]] = []
    remove_parents: list[list[str]] = []
    add_dependencies: list[list[str]] = []
    remove_dependencies: list[list[str]] = []
    unmanaged_relationships = 0
    if include_relationships:
        numeric_to_task = {issue["id"]: task_id for task_id, issue in snapshot.issue_by_task.items()}
        desired_parent = {(issue["parent"], issue["id"]) for issue in manifest["issues"] if issue.get("parent")}
        existing_parent: set[tuple[str, str]] = set()
        for parent, numeric_children in snapshot.subissues.items():
            for numeric_child in numeric_children:
                child = numeric_to_task.get(numeric_child)
                if child is None:
                    unmanaged_relationships += 1
                else:
                    existing_parent.add((parent, child))
        add_parents = [list(edge) for edge in sorted(desired_parent - existing_parent) if all(task in snapshot.issue_by_task for task in edge)]
        remove_parents = [list(edge) for edge in sorted(existing_parent - desired_parent)]

        desired_dependencies = {(issue["id"], blocker) for issue in manifest["issues"] for blocker in issue["depends_on"]}
        existing_dependencies: set[tuple[str, str]] = set()
        for task_id, numeric_blockers in snapshot.blocked_by.items():
            for numeric_blocker in numeric_blockers:
                blocker = numeric_to_task.get(numeric_blocker)
                if blocker is None:
                    unmanaged_relationships += 1
                else:
                    existing_dependencies.add((task_id, blocker))
        add_dependencies = [list(edge) for edge in sorted(desired_dependencies - existing_dependencies) if all(task in snapshot.issue_by_task for task in edge)]
        remove_dependencies = [list(edge) for edge in sorted(existing_dependencies - desired_dependencies)]

    desired_parent_count = sum(issue.get("parent") is not None for issue in manifest["issues"])
    desired_dependency_count = sum(len(issue["depends_on"]) for issue in manifest["issues"])
    return {
        "desired": {
            "issues": len(manifest["issues"]), "milestones": len(manifest["milestones"]),
            "phases": len({issue["phase"] for issue in manifest["issues"]}),
            "parent_edges": desired_parent_count, "dependency_edges": desired_dependency_count,
        },
        "live": {
            "active_issues": sum(issue["id"] in snapshot.issue_by_task for issue in manifest["issues"]),
            "retired_issues": sum(issue["id"] in snapshot.issue_by_task for issue in manifest["retired_issues"]),
            "milestones": len(snapshot.milestones),
        },
        "mutations": {
            "create_labels": create_labels, "update_labels": update_labels,
            "create_milestones": [item["key"] for item in create_milestones], "update_milestones": update_milestones,
            "create_issues": create_issues, "update_issues": update_issues, "retire_issues": retire_issues,
            "add_parent_edges": add_parents, "remove_parent_edges": remove_parents,
            "add_dependency_edges": add_dependencies, "remove_dependency_edges": remove_dependencies,
        },
        "warnings": {"unexpected_closed": unexpected_closed, "unmanaged_relationships": unmanaged_relationships},
        "_create_milestones": create_milestones,
    }


def mutation_counts(plan: dict[str, Any]) -> dict[str, int]:
    return {name: len(items) for name, items in plan["mutations"].items()}


def mutation_total(plan: dict[str, Any]) -> int:
    return sum(mutation_counts(plan).values())


def public_plan(plan: dict[str, Any]) -> dict[str, Any]:
    return {
        "desired": plan["desired"], "live": plan["live"],
        "mutation_counts": mutation_counts(plan),
        "mutations": {
            "create_milestones": plan["mutations"]["create_milestones"],
            "create_issues": plan["mutations"]["create_issues"],
            "update_issue_ids": [item["id"] for item in plan["mutations"]["update_issues"]],
            "retire_issue_ids": [item["id"] for item in plan["mutations"]["retire_issues"]],
            "add_parent_edges": plan["mutations"]["add_parent_edges"],
            "add_dependency_edges": plan["mutations"]["add_dependency_edges"],
        },
        "warnings": plan["warnings"],
    }


def progress(action: str, current: int, total: int) -> None:
    if current == 1 or current == total or current % 10 == 0:
        print(f"{action}: {current}/{total}", flush=True)


def write_gh(method: str, path: str, payload: dict[str, Any] | None = None) -> Any:
    result = run_gh(method, path, payload)
    time.sleep(WRITE_DELAY_SECONDS)
    return result


def apply_labels_and_milestones(repository: str, plan: dict[str, Any]) -> None:
    for item in plan["mutations"]["create_labels"]:
        write_gh("POST", f"repos/{repository}/labels", item)
    for item in plan["mutations"]["update_labels"]:
        write_gh("PATCH", f"repos/{repository}/labels/{item['current_name']}", item["payload"])
    for item in plan["_create_milestones"]:
        write_gh("POST", f"repos/{repository}/milestones", {
            "title": item["title"],
            "description": f"{item['focus']} Exit gate: {item['exit_criteria']}",
            "due_on": f"{item['due_on']}T23:59:59Z",
            "state": "open",
        })
    for item in plan["mutations"]["update_milestones"]:
        write_gh("PATCH", f"repos/{repository}/milestones/{item['number']}", item["payload"])


def apply_issues(repository: str, manifest: dict[str, Any], snapshot: Snapshot, plan: dict[str, Any]) -> None:
    milestone_by_title = {item["title"]: item for item in snapshot.milestones}
    milestones = {item["key"]: item for item in manifest["milestones"]}
    by_id = {item["id"]: item for item in manifest["issues"]}
    updates = plan["mutations"]["update_issues"]
    for index, item in enumerate(updates, start=1):
        write_gh("PATCH", f"repos/{repository}/issues/{item['number']}", item["payload"])
        progress("update issues", index, len(updates))
    retired = plan["mutations"]["retire_issues"]
    for index, item in enumerate(retired, start=1):
        write_gh("PATCH", f"repos/{repository}/issues/{item['number']}", item["payload"])
        progress("retire consolidated issues", index, len(retired))
    # Creation is deliberately last: adoption and sanitization of existing
    # public records is the first recovery priority after a partial publish.
    creates = plan["mutations"]["create_issues"]
    for index, task_id in enumerate(creates, start=1):
        desired = by_id[task_id]
        milestone_spec = milestones[desired["quarter"]]
        milestone = milestone_by_title[milestone_spec["title"]]
        write_gh("POST", f"repos/{repository}/issues", {
            "title": desired["title"], "body": public_body(desired, milestone_spec),
            "milestone": milestone["number"], "labels": desired["labels"],
        })
        progress("create issues", index, len(creates))


def apply_relationships(repository: str, snapshot: Snapshot, plan: dict[str, Any]) -> None:
    issue_by_task = snapshot.issue_by_task
    operations = [
        ("remove parent edges", plan["mutations"]["remove_parent_edges"]),
        ("add parent edges", plan["mutations"]["add_parent_edges"]),
        ("remove dependency edges", plan["mutations"]["remove_dependency_edges"]),
        ("add dependency edges", plan["mutations"]["add_dependency_edges"]),
    ]
    for action, edges in operations:
        for index, (source, target) in enumerate(edges, start=1):
            source_issue = issue_by_task[source]
            target_issue = issue_by_task[target]
            if action == "remove parent edges":
                write_gh("DELETE", f"repos/{repository}/issues/{source_issue['number']}/sub_issue", {"sub_issue_id": target_issue["id"]})
            elif action == "add parent edges":
                write_gh("POST", f"repos/{repository}/issues/{source_issue['number']}/sub_issues", {"sub_issue_id": target_issue["id"], "replace_parent": False})
            elif action == "remove dependency edges":
                write_gh("DELETE", f"repos/{repository}/issues/{source_issue['number']}/dependencies/blocked_by/{target_issue['id']}")
            else:
                write_gh("POST", f"repos/{repository}/issues/{source_issue['number']}/dependencies/blocked_by", {"issue_id": target_issue["id"]})
            progress(action, index, len(edges))


def verify_live(manifest: dict[str, Any], snapshot: Snapshot, plan: dict[str, Any]) -> dict[str, Any]:
    if mutation_total(plan) != 0:
        raise ValueError(f"live roadmap still needs mutations: {mutation_counts(plan)}")
    active_ids = {issue["id"] for issue in manifest["issues"]}
    retired_ids = {issue["id"] for issue in manifest["retired_issues"]}
    if set(snapshot.issue_by_task) != active_ids | retired_ids:
        raise ValueError("live active and retired issue markers do not match the manifest")
    if len(snapshot.milestones) != len(manifest["milestones"]):
        raise ValueError("live milestone count does not match the 20-quarter program")
    desired_parent = sum(issue.get("parent") is not None for issue in manifest["issues"])
    desired_dependencies = sum(len(issue["depends_on"]) for issue in manifest["issues"])
    managed_numeric = {snapshot.issue_by_task[task_id]["id"] for task_id in active_ids}
    actual_parent = sum(len(children & managed_numeric) for children in snapshot.subissues.values())
    actual_dependencies = sum(len(blockers & managed_numeric) for blockers in snapshot.blocked_by.values())
    if actual_parent != desired_parent or actual_dependencies != desired_dependencies:
        raise ValueError("live native relationship counts do not match the manifest")
    unsafe_patterns = ("forge-task:v1", "gpt-", "assigned:", "routing metadata", ".forge/")
    for issue in snapshot.issues:
        body = (issue.get("body") or "").lower()
        if any(pattern in body for pattern in unsafe_patterns):
            raise ValueError(f"public issue or pull request #{issue['number']} still contains private task metadata")
    closed = sum(issue.get("state") == "closed" for issue in snapshot.issue_by_task.values())
    return {
        "active_issues": len(active_ids), "retired_issues": len(retired_ids),
        "total_managed_issues": len(snapshot.issue_by_task), "milestones": len(snapshot.milestones),
        "phases": len({issue["phase"] for issue in manifest["issues"]}),
        "parent_edges": actual_parent, "dependency_edges": actual_dependencies,
        "closed_issues": closed, "mutation_count": 0,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    parser.add_argument("--repository", help="OWNER/REPO; defaults to the manifest repository")
    parser.add_argument("--validate-only", action="store_true", help="validate the tracked manifest without GitHub access")
    parser.add_argument("--apply", action="store_true", help="perform and verify planned GitHub mutations")
    args = parser.parse_args()

    manifest = load_manifest(args.manifest)
    local_summary = {
        "active_issues": len(manifest["issues"]), "retired_issues": len(manifest["retired_issues"]),
        "milestones": len(manifest["milestones"]),
        "phases": len({item["phase"] for item in manifest["issues"]}),
        "parent_edges": sum(item.get("parent") is not None for item in manifest["issues"]),
        "dependency_edges": sum(len(item["depends_on"]) for item in manifest["issues"]),
    }
    if args.validate_only:
        print(json.dumps({"valid": True, **local_summary}, indent=2))
        return 0

    repository = args.repository or manifest["program"]["repository"]
    initial_snapshot = load_snapshot(repository, manifest, relationships=True)
    initial_plan = build_plan(manifest, initial_snapshot, include_relationships=True)
    print(json.dumps({"initial_plan": public_plan(initial_plan)}, indent=2), flush=True)
    if not args.apply:
        return 0

    apply_labels_and_milestones(repository, initial_plan)
    issue_snapshot = load_snapshot(repository, manifest, relationships=False)
    issue_plan = build_plan(manifest, issue_snapshot, include_relationships=False)
    apply_issues(repository, manifest, issue_snapshot, issue_plan)

    relationship_snapshot = load_snapshot(repository, manifest, relationships=True)
    relationship_plan = build_plan(manifest, relationship_snapshot, include_relationships=True)
    apply_relationships(repository, relationship_snapshot, relationship_plan)

    final_snapshot = load_snapshot(repository, manifest, relationships=True)
    final_plan = build_plan(manifest, final_snapshot, include_relationships=True)
    verified = verify_live(manifest, final_snapshot, final_plan)
    print(json.dumps({"verified": verified, "second_plan": public_plan(final_plan)}, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
