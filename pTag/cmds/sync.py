import json
from datetime import datetime
from pathlib import Path

from helper.colors import YELLOW, RED, RESET, BOLD, DIM, BLUE, GREEN, MAGENTA, CYAN
from helper.db import load_db, save_db
from helper.git import get_git_status
from helper.logger import info, ok, warn, _vis
from helper.project import last_modified_file, node_modules_size, within_days, time_ago, fmt_size


def cmd_sync(args):
    db       = load_db()
    projects = db["projects"]
    fmt      = args.format
    db_dirty = False

    if not projects:
        if fmt == "json":
            print(json.dumps({"missing": [], "projects": [], "summary": {}}))
        else:
            info("No projects tracked yet. Use 'add' to get started.")
        return

    archive_days      = args.archive_days
    deep_archive_days = getattr(args, "deep_archive_days", None)

    # ── 1. Find missing ──────────────────────────────────────────────────────
    # Deep-archived projects have no folder on disk by design — exclude them.
    missing_paths = [
        p for p, v in projects.items()
        if not Path(p).exists() and not v.get("deep_archived", False)
    ]

    if fmt != "json":
        if missing_paths:
            print(f"\n{YELLOW}Missing projects (folder no longer exists):{RESET}")
            for p in missing_paths:
                print(f"  {RED}✖{RESET}  {BOLD}{projects[p]['name']}{RESET}  {DIM}{p}{RESET}")
            print()
            try:
                answer = input("Remove them from the database? [y/N] ").strip().lower()
            except (EOFError, KeyboardInterrupt):
                print()
                info("Aborted.")
                return
            if answer == "y":
                for p in missing_paths:
                    del db["projects"][p]
                db_dirty = True
                ok(f"Removed {len(missing_paths)} project(s) from the database.")
            else:
                info("Kept missing entries.")
        else:
            ok("All projects exist on disk.")

        print(f"\n{BOLD}Scanning projects...{RESET}\n")

    # ── 2. Collect rows ──────────────────────────────────────────────────────
    # Include deep-archived entries even though their folder is gone.
    live_projects = {
        p: v for p, v in db["projects"].items()
        if Path(p).exists() or v.get("deep_archived", False)
    }
    rows = []

    for path, proj in sorted(live_projects.items(), key=lambda x: x[1]["name"].lower()):
        archived      = proj.get("archived", False)
        deep_archived = proj.get("deep_archived", False)
        exists        = Path(path).exists()

        # Deep-archived: no filesystem scan, no git scan
        if deep_archived:
            zip_path    = proj.get("zip_path")
            zip_size    = None
            if zip_path:
                zp = Path(zip_path)
                if zp.exists():
                    try:
                        zip_size = zp.stat().st_size
                    except OSError:
                        pass

            rows.append({
                "name":             proj["name"],
                "path":             path,
                "exists":           False,
                "archived":         True,
                "deep_archived":    True,
                "archived_at":      proj.get("archived_at"),
                "deep_archived_at": proj.get("deep_archived_at"),
                "zip_path":         zip_path,
                "zip_size":         zip_size,
                "added":            proj.get("added"),
                "updated":          proj.get("updated"),
                "last_file":        None,
                "last_file_at":     None,
                "node_modules_size": None,
                "stale":            False,
                "has_dirty":        False,
                "has_unpushed":     False,
                "git":              None,
            })
            continue

        lf_path, lf_time = last_modified_file(path)
        gs               = get_git_status(path)
        nm_size          = node_modules_size(path)

        rows.append({
            "name":             proj["name"],
            "path":             path,
            "exists":           True,
            "archived":         archived,
            "deep_archived":    False,
            "archived_at":      proj.get("archived_at"),
            "deep_archived_at": None,
            "zip_path":         None,
            "zip_size":         None,
            "added":            proj.get("added"),
            "updated":          proj.get("updated"),
            "last_file":        str(lf_path.relative_to(path)) if lf_path else None,
            "last_file_at":     lf_time.isoformat() if lf_time else None,
            "node_modules_size": nm_size,
            "stale":            (
                    not archived and lf_time is not None
                    and not within_days(lf_time, archive_days)
            ),
            "has_dirty":        bool(gs["staged"] or gs["unstaged"] or gs["untracked"]),
            "has_unpushed":     bool(gs["unpushed"]),
            "git": {
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
            },
        })

    # ── JSON output ──────────────────────────────────────────────────────────
    if fmt == "json":
        missing_list = [{"name": projects[p]["name"], "path": p} for p in missing_paths]
        print(json.dumps({
            "missing": missing_list,
            "projects": rows,
            "summary": {
                "total":         len(rows),
                "stale":         sum(1 for r in rows if r["stale"]),
                "dirty":         sum(1 for r in rows if r["has_dirty"]),
                "unpushed":      sum(1 for r in rows if r["has_unpushed"]),
                "archived":      sum(1 for r in rows if r["archived"] and not r["deep_archived"]),
                "deep_archived": sum(1 for r in rows if r["deep_archived"]),
            },
        }, indent=2))
        if db_dirty:
            save_db(db)
        return

    # ── Normal table output ──────────────────────────────────────────────────
    if not rows:
        info("No projects to display.")
        if db_dirty:
            save_db(db)
        return

    col_name   = max(_vis(r["name"]) for r in rows) + 2
    col_edited = 14
    col_commit = 14

    print(f"  {BOLD}{'Name':<{col_name}}{'Last edit':<{col_edited}}{'Last commit':<{col_commit}}Status{RESET}")
    print(f"  {'─' * (col_name + col_edited + col_commit + 26)}")

    for r in rows:
        g = r["git"] or {}

        if r["deep_archived"]:
            # ── Deep-archived row ────────────────────────────────────────────
            da_str   = (
                f"deep-archived {time_ago(datetime.fromisoformat(r['deep_archived_at']))}"
                if r.get("deep_archived_at") else "deep-archived"
            )
            zip_info = ""
            if r["zip_path"]:
                size_part = f"  {fmt_size(r['zip_size'])}" if r["zip_size"] else ""
                zip_info  = f"  {DIM}{r['zip_path']}{size_part}{RESET}"

            np = col_name - _vis(r["name"])
            print(
                f"  {BOLD}{r['name']}{RESET}{' ' * np}"
                f"{DIM}{'—':<{col_edited}}{'—':<{col_commit}}{RESET}"
                f"  {MAGENTA}{da_str}{RESET}{zip_info}"
            )
        else:
            # ── Normal row ───────────────────────────────────────────────────
            lf_str = (
                time_ago(datetime.fromisoformat(r["last_file_at"]))
                if r["last_file_at"] else f"{DIM}n/a{RESET}"
            )
            lc_str = (
                time_ago(datetime.fromisoformat(g["last_commit"]))
                if g.get("last_commit") else f"{DIM}n/a{RESET}"
            )

            flags = []
            if r["archived"]:
                flags.append(f"{BLUE}archived{RESET}")
            elif r["stale"]:
                flags.append(f"{YELLOW}stale >{archive_days}d{RESET}")
            if r["has_dirty"]:    flags.append(f"{RED}dirty{RESET}")
            if r["has_unpushed"]: flags.append(f"{YELLOW}unpushed{RESET}")
            if not flags:         flags.append(f"{GREEN}clean{RESET}")

            np = col_name - _vis(r["name"])
            ep = col_edited - _vis(lf_str)
            cp = col_commit - _vis(lc_str)

            print(
                f"  {BOLD}{r['name']}{RESET}{' ' * np}"
                f"{lf_str}{' ' * ep}{lc_str}{' ' * cp}"
                + "  ".join(flags)
            )

    # ── Summary ──────────────────────────────────────────────────────────────
    n_stale         = sum(1 for r in rows if r["stale"])
    n_dirty         = sum(1 for r in rows if r["has_dirty"])
    n_unpushed      = sum(1 for r in rows if r["has_unpushed"])
    n_archived      = sum(1 for r in rows if r["archived"] and not r["deep_archived"])
    n_deep_archived = sum(1 for r in rows if r["deep_archived"])

    print(f"\n  {DIM}{'─' * 40}{RESET}")
    summary_parts = [
        f"{len(rows)} total",
        f"{YELLOW}{n_stale} stale{RESET}",
        f"{RED}{n_dirty} dirty{RESET}",
        f"{YELLOW}{n_unpushed} unpushed{RESET}",
        f"{BLUE}{n_archived} archived{RESET}",
        f"{MAGENTA}{n_deep_archived} deep-archived{RESET}",
    ]
    print("  " + "   ".join(summary_parts))

    if n_stale:
        print(f"\n  {DIM}Tip: run 'ptag check' to archive stale projects and clean node_modules.{RESET}")
    if n_deep_archived:
        print(f"  {DIM}Tip: run 'ptag restore <name>' to unzip and reactivate a deep-archived project.{RESET}")

    if db_dirty:
        save_db(db)
