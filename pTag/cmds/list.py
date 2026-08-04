import json
from datetime import datetime
from pathlib import Path

from helper.colors import BLUE, GREEN, RED, RESET, BOLD, DIM, YELLOW, MAGENTA, CYAN
from helper.db import load_db
from helper.git import get_git_status
from helper.logger import info
from helper.project import time_ago, last_modified_file, fmt_size


def _collect(path: str, proj: dict) -> dict:
    exists        = Path(path).exists()
    archived      = proj.get("archived", False)
    deep_archived = proj.get("deep_archived", False)

    if not exists:
        return {
            "name":          proj["name"],
            "path":          path,
            "exists":        False,
            "archived":      archived,
            "deep_archived": deep_archived,
            "archived_at":      proj.get("archived_at"),
            "deep_archived_at": proj.get("deep_archived_at"),
            "zip_path":      proj.get("zip_path"),
            "added":         proj.get("added"),
            "updated":       proj.get("updated"),
            "last_file_at":  None,
            "git":           None,
            "has_dirty":     False,
            "has_unpushed":  False,
        }

    _, lf_time = last_modified_file(path)
    gs = get_git_status(path)

    return {
        "name":          proj["name"],
        "path":          path,
        "exists":        True,
        "archived":      archived,
        "deep_archived": deep_archived,
        "archived_at":      proj.get("archived_at"),
        "deep_archived_at": proj.get("deep_archived_at"),
        "zip_path":      proj.get("zip_path"),
        "added":         proj.get("added"),
        "updated":       proj.get("updated"),
        "last_file_at":  lf_time.isoformat() if lf_time else None,
        "git":           gs,
        "has_dirty":     bool(gs["staged"] or gs["unstaged"] or gs["untracked"]),
        "has_unpushed":  bool(gs["unpushed"]),
    }


def _badges(d: dict) -> list[str]:
    badges = []
    if d["has_unpushed"]:
        badges.append(f"{YELLOW}↑ unpushed{RESET}")
    if d["has_dirty"]:
        g = d["git"] or {}
        parts = []
        if g.get("staged"):    parts.append(f"{len(g['staged'])} staged")
        if g.get("unstaged"):  parts.append(f"{len(g['unstaged'])} unstaged")
        if g.get("untracked"): parts.append(f"{len(g['untracked'])} untracked")
        badges.append(f"{RED}● {', '.join(parts)}{RESET}")
    return badges


def _time_ago_str(iso: str | None) -> str:
    if not iso:
        return "?"
    return time_ago(datetime.fromisoformat(iso))


def _print_row(d: dict):
    """Full detail row — used for Needs Attention + Active groups."""
    g = d["git"] or {}

    edit_str = (
        f"{DIM}edited {time_ago(datetime.fromisoformat(d['last_file_at']))}{RESET}"
        if d["last_file_at"] else f"{DIM}no files{RESET}"
    )

    if g.get("last_commit"):
        commit_str = f"{DIM}commit {time_ago(g['last_commit'])}  {g.get('last_sha', '')}{RESET}"
        branch_str = f"  {MAGENTA}{g.get('branch', '')}{RESET}"
    elif g.get("is_git"):
        commit_str = f"{DIM}no commits{RESET}"
        branch_str = f"  {MAGENTA}{g.get('branch', '')}{RESET}"
    else:
        commit_str = f"{DIM}no git{RESET}"
        branch_str = ""

    badges    = _badges(d)
    badge_str = f"  {'  '.join(badges)}" if badges else ""

    print(f"    {BOLD}{d['name']}{RESET}{branch_str}{badge_str}")
    print(f"    {DIM}{d['path']}{RESET}")
    print(f"    {edit_str}   {commit_str}")


def _print_compact(d: dict):
    """Compact row — used for Archived + Missing groups."""
    g          = d["git"] or {}
    branch_str = f"  {MAGENTA}{g.get('branch', '')}{RESET}" if g.get("branch") else ""
    edit_str   = (
        f"  {DIM}edited {time_ago(datetime.fromisoformat(d['last_file_at']))}{RESET}"
        if d["last_file_at"] else ""
    )
    archived_str = (
        f"  {DIM}archived {_time_ago_str(d['archived_at'])}{RESET}"
        if d.get("archived_at") else ""
    )
    print(f"    {BOLD}{d['name']}{RESET}{branch_str}{edit_str}{archived_str}")
    print(f"    {DIM}{d['path']}{RESET}")


def _print_deep_archived(d: dict):
    """Row for deep-archived projects — shows zip info."""
    zip_path = d.get("zip_path")
    da_str   = f"  {DIM}deep-archived {_time_ago_str(d['deep_archived_at'])}{RESET}" if d.get("deep_archived_at") else ""

    # zip size if accessible
    zip_size_str = ""
    if zip_path:
        zp = Path(zip_path)
        if zp.exists():
            try:
                zip_size_str = f"  {DIM}({fmt_size(zp.stat().st_size)}){RESET}"
            except OSError:
                pass

    print(f"    {BOLD}{d['name']}{RESET}{da_str}")
    print(f"    {DIM}{d['path']}{RESET}")
    if zip_path:
        print(f"    {MAGENTA}zip{RESET}  {DIM}{zip_path}{RESET}{zip_size_str}")
    else:
        print(f"    {DIM}(no zip path recorded){RESET}")


def cmd_list(args):
    db       = load_db()
    projects = db["projects"]
    fmt      = args.format

    if not projects:
        if fmt == "json":
            print("[]")
        else:
            info("No projects tracked yet. Use 'add' to get started.")
        return

    sorted_projects = sorted(projects.items(), key=lambda x: x[1]["name"].lower())

    # ── Collect data ─────────────────────────────────────────────────────────
    collected = [_collect(path, proj) for path, proj in sorted_projects]

    if fmt == "json":
        print(json.dumps(collected, indent=2, default=str))
        return

    # ── Group into 5 buckets ─────────────────────────────────────────────────
    deep_archived   = [d for d in collected if d["deep_archived"]]
    archived        = [d for d in collected if d["archived"] and not d["deep_archived"] and d["exists"]]
    active          = [d for d in collected if d["exists"] and not d["archived"] and not d["deep_archived"]]
    missing         = [d for d in collected if not d["exists"] and not d["deep_archived"]]

    needs_attention = [d for d in active if d["has_dirty"] or d["has_unpushed"]]
    clean           = [d for d in active if not d["has_dirty"] and not d["has_unpushed"]]

    # ── Print helpers ────────────────────────────────────────────────────────
    def section_header(label: str, color: str, count: int):
        print(f"\n  {color}{BOLD}{label}{RESET}  {DIM}({count}){RESET}")
        print(f"  {'─' * 50}")

    if needs_attention:
        section_header("Needs Attention", RED, len(needs_attention))
        for d in needs_attention:
            _print_row(d)
            print()

    if clean:
        section_header("Active", GREEN, len(clean))
        for d in clean:
            _print_row(d)
            print()

    if archived:
        section_header("Archived", BLUE, len(archived))
        for d in archived:
            _print_compact(d)
            print()

    if deep_archived:
        section_header("Deep Archived", MAGENTA, len(deep_archived))
        for d in deep_archived:
            _print_deep_archived(d)
            print()

    if missing:
        section_header("Missing", RED, len(missing))
        for d in missing:
            _print_compact(d)
            print()

    # ── Summary line ─────────────────────────────────────────────────────────
    total = len(collected)
    parts = []
    if needs_attention: parts.append(f"{RED}{len(needs_attention)} need attention{RESET}")
    if clean:           parts.append(f"{GREEN}{len(clean)} clean{RESET}")
    if archived:        parts.append(f"{BLUE}{len(archived)} archived{RESET}")
    if deep_archived:   parts.append(f"{MAGENTA}{len(deep_archived)} deep-archived{RESET}")
    if missing:         parts.append(f"{RED}{len(missing)} missing{RESET}")

    print(f"  {DIM}{total} total  —  {RESET}" + f"  {DIM}·{RESET}  ".join(parts))
