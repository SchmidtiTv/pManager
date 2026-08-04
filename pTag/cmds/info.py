import json
import sys
from datetime import datetime
from pathlib import Path

from helper.colors import BOLD, RESET, BLUE, DIM, RED, GREEN, YELLOW, MAGENTA, CYAN
from helper.db import load_db, resolve
from helper.git import get_git_status
from helper.logger import err
from helper.project import last_modified_file, node_modules_size, time_ago, fmt_size


def _field(label: str, value: str, width: int = 16):
    print(f"  {DIM}{label:<{width}}{RESET}{value}")


def cmd_info(args):
    db   = load_db()
    fmt  = args.format

    # ── Lookup: by path or by name ───────────────────────────────────────────
    path_key = resolve(args.path)
    proj     = db["projects"].get(path_key)

    if proj is None:
        matches = [
            (k, v) for k, v in db["projects"].items()
            if v["name"].lower() == args.path.lower()
        ]
        if len(matches) == 1:
            path_key, proj = matches[0]
        elif len(matches) > 1:
            err(f"Ambiguous name '{args.path}' — matched multiple projects:")
            for k, v in matches:
                print(f"  {DIM}{k}{RESET}")
            sys.exit(1)
        else:
            err(f"Project not tracked: {args.path}")
            sys.exit(1)

    name          = proj["name"]
    archived      = proj.get("archived", False)
    deep_archived = proj.get("deep_archived", False)
    exists        = Path(path_key).exists()

    # ── JSON output ──────────────────────────────────────────────────────────
    if fmt == "json":
        payload = {
            "name":             name,
            "path":             path_key,
            "exists":           exists,
            "archived":         archived,
            "deep_archived":    deep_archived,
            "archived_at":      proj.get("archived_at"),
            "deep_archived_at": proj.get("deep_archived_at"),
            "zip_path":         proj.get("zip_path"),
            "restored_at":      proj.get("restored_at"),
            "restored_to":      proj.get("restored_to"),
            "added":            proj.get("added"),
            "updated":          proj.get("updated"),
        }

        # Enrich with live data when folder is present
        if exists:
            _, lf_time = last_modified_file(path_key)
            gs         = get_git_status(path_key)
            nm_size    = node_modules_size(path_key)
            payload["last_file_at"]     = lf_time.isoformat() if lf_time else None
            payload["node_modules_size"] = nm_size
            payload["git"] = {
                "is_repo":      gs["is_git"],
                "branch":       gs["branch"],
                "last_commit":  gs["last_commit"].isoformat() if gs["last_commit"] else None,
                "last_sha":     gs["last_sha"],
                "last_subject": gs["last_subject"],
                "last_author":  gs["last_author"],
                "unpushed":     gs["unpushed"],
                "staged":       gs["staged"],
                "unstaged":     gs["unstaged"],
                "untracked":    gs["untracked"],
            }

        print(json.dumps(payload, indent=2))
        return

    # ── Human-readable output ────────────────────────────────────────────────
    if deep_archived:
        state_badge = f"{MAGENTA}deep-archived{RESET}"
    elif archived:
        state_badge = f"{BLUE}archived{RESET}"
    elif exists:
        state_badge = f"{GREEN}active{RESET}"
    else:
        state_badge = f"{RED}MISSING{RESET}"

    print(f"\n  {BOLD}{CYAN}{name}{RESET}  {state_badge}")
    print(f"  {'─' * 55}")

    _field("Path",    path_key)
    _field("Added",   proj.get("added",   "?"))
    _field("Updated", proj.get("updated", "?"))

    if proj.get("restored_at"):
        _field("Restored at", proj["restored_at"])
    if proj.get("restored_to"):
        _field("Restored to", proj["restored_to"])

    # ── State-specific block ─────────────────────────────────────────────────
    if deep_archived:
        print()
        da_since = proj.get("deep_archived_at", "?")
        da_ago   = (
            f"  {DIM}({time_ago(datetime.fromisoformat(da_since))}){RESET}"
            if da_since != "?" else ""
        )
        _field("Deep-archived", f"{da_since}{da_ago}")

        zip_path = proj.get("zip_path")
        if zip_path:
            zp       = Path(zip_path)
            zip_ok   = zp.exists()
            size_str = ""
            if zip_ok:
                try:
                    size_str = f"  {DIM}({fmt_size(zp.stat().st_size)}){RESET}"
                except OSError:
                    pass
            status_icon = f"{GREEN}✔{RESET}" if zip_ok else f"{RED}✖ not found{RESET}"
            _field("Zip",    f"{zip_path}{size_str}  {status_icon}")
        else:
            _field("Zip", f"{RED}(none recorded){RESET}")

        print()
        print(f"  {DIM}Run 'ptag restore {name}' to unzip and reactivate this project.{RESET}")

    elif archived:
        ar_since = proj.get("archived_at", "?")
        ar_ago   = (
            f"  {DIM}({time_ago(datetime.fromisoformat(ar_since))}){RESET}"
            if ar_since != "?" else ""
        )
        print()
        _field("Archived", f"{ar_since}{ar_ago}")

        if exists:
            _, lf_time = last_modified_file(path_key)
            nm_size    = node_modules_size(path_key)

            if lf_time:
                _field("Last file edit", f"{time_ago(lf_time)}  {DIM}({lf_time.strftime('%Y-%m-%d %H:%M')} UTC){RESET}")

            nm_str = (
                f"{fmt_size(nm_size)}  {DIM}(present){RESET}"
                if nm_size is not None else f"{DIM}absent{RESET}"
            )
            _field("node_modules", nm_str)

    elif exists:
        # ── Active project: full git + file details ──────────────────────────
        print()
        _, lf_time = last_modified_file(path_key)
        gs         = get_git_status(path_key)
        nm_size    = node_modules_size(path_key)

        if lf_time:
            _field("Last file edit",
                   f"{time_ago(lf_time)}  {DIM}({lf_time.strftime('%Y-%m-%d %H:%M')} UTC){RESET}")
        else:
            _field("Last file edit", f"{DIM}no files found{RESET}")

        nm_str = (
            f"{fmt_size(nm_size)}  {DIM}(present){RESET}"
            if nm_size is not None else f"{DIM}absent{RESET}"
        )
        _field("node_modules", nm_str)

        print()
        if not gs["is_git"]:
            _field("Git", f"{DIM}not a git repository{RESET}")
        else:
            _field("Branch",      f"{MAGENTA}{gs['branch']}{RESET}")
            if gs["last_commit"]:
                _field("Last commit",
                       f"{time_ago(gs['last_commit'])}  "
                       f"{DIM}{gs['last_sha']}  {gs['last_subject']}  ({gs['last_author']}){RESET}")
            else:
                _field("Last commit", f"{YELLOW}no commits yet{RESET}")

            def _git_group(label: str, items: list, color: str):
                if not items:
                    return
                _field(label, f"{color}{len(items)} file(s){RESET}")
                for item in items[:5]:
                    print(f"  {' ' * 16}{DIM}• {item}{RESET}")
                if len(items) > 5:
                    print(f"  {' ' * 16}{DIM}… and {len(items) - 5} more{RESET}")

            _git_group("Unpushed",  gs["unpushed"],  YELLOW)
            _git_group("Staged",    gs["staged"],    GREEN)
            _git_group("Unstaged",  gs["unstaged"],  YELLOW)
            _git_group("Untracked", gs["untracked"], RED)

            if not any([gs["unpushed"], gs["staged"], gs["unstaged"], gs["untracked"]]):
                _field("Working tree", f"{GREEN}clean{RESET}")

    else:
        # Missing — not deep_archived but folder gone
        print()
        print(f"  {RED}✖  Project folder not found on disk.{RESET}")
        print(f"  {DIM}Run 'ptag sync' to clean up missing entries.{RESET}")

    print()
