import json
import shutil
import sys
import tempfile
import zipfile
from datetime import datetime
from pathlib import Path

from helper.colors import RESET, BOLD, GREEN, YELLOW, RED, CYAN, DIM, MAGENTA, BLUE
from helper.db import load_db, resolve, save_db
from helper.git import get_git_status
from helper.logger import err, info, ok, warn
from helper.prettyPrint import _print_group, _print_header
from helper.project import last_modified_file, node_modules_size, within_days, time_ago, fmt_size

# ── Deep archive helpers ───────────────────────────────────────────────────────

DEEP_ARCHIVE_SKIP = {"node_modules", ".git", ".next", "dist", "build", ".cache", "coverage"}


def _zip_project(src: Path, dest_zip: Path) -> tuple[bool, str]:
    """
    Zip `src` into `dest_zip` safely:
      1. Write into a sibling temp file first
      2. Verify every entry can be read back
      3. Only rename to final path on success

    Returns (ok, error_message).
    """
    tmp = dest_zip.with_suffix(".tmp.zip")
    try:
        # ── Step 1: write ────────────────────────────────────────────────────
        with zipfile.ZipFile(tmp, "w", compression=zipfile.ZIP_DEFLATED, allowZip64=True) as zf:
            for root, dirs, files in src.rglob("") if False else _walk(src):
                for fpath in files:
                    arcname = fpath.relative_to(src.parent)  # keep top-level folder name
                    zf.write(fpath, arcname)

        # ── Step 2: verify ───────────────────────────────────────────────────
        bad = _verify_zip(tmp)
        if bad:
            tmp.unlink(missing_ok=True)
            return False, f"Verification failed for: {bad}"

        # ── Step 3: commit ───────────────────────────────────────────────────
        tmp.replace(dest_zip)
        return True, ""

    except Exception as ex:
        tmp.unlink(missing_ok=True)
        return False, str(ex)


def _walk(src: Path):
    """Yield lists of file Paths under src, skipping unwanted dirs."""
    import os
    for root, dirs, files in os.walk(src):
        dirs[:] = [d for d in dirs if d not in DEEP_ARCHIVE_SKIP]
        yield None, None, [Path(root) / f for f in files]


def _verify_zip(zip_path: Path) -> str | None:
    """Return the first bad filename, or None if all entries are readable."""
    try:
        with zipfile.ZipFile(zip_path, "r") as zf:
            bad = zf.testzip()
            return bad  # None = all good
    except Exception as ex:
        return str(ex)


def deep_archive_project(path: str, yes: bool, zip_dir: Path | None = None) -> tuple[bool, str]:
    """
    Safely zip a project folder.
    - Writes to <zip_dir>/<name>.zip  (default: same parent as project)
    - Verifies the zip before doing anything irreversible
    - Only removes the original folder after the zip is confirmed good
      (and only if the user agrees)

    Returns (success, message).
    """
    src      = Path(path)
    zip_dir  = zip_dir or src.parent
    zip_dir.mkdir(parents=True, exist_ok=True)
    dest_zip = zip_dir / f"{src.name}.zip"

    # Don't overwrite an existing archive without asking
    if dest_zip.exists():
        return False, f"Archive already exists: {dest_zip}"

    success, errmsg = _zip_project(src, dest_zip)
    if not success:
        return False, errmsg

    # Zip is verified — now optionally remove the source
    zip_size = dest_zip.stat().st_size
    return True, str(dest_zip)


# ── Main command ───────────────────────────────────────────────────────────────

def cmd_check(args):
    db       = load_db()
    projects = db["projects"]
    fmt      = args.format
    db_dirty = False

    if args.path:
        path = resolve(args.path)
        if path not in projects:
            err(f"Project not tracked: {path}")
            sys.exit(1)
        projects = {path: projects[path]}

    if not projects:
        if fmt == "json":
            print("[]")
        else:
            info("No projects tracked yet. Use 'add' to get started.")
        return

    show_all          = args.all
    edited_days       = None if show_all else args.edited_days
    commit_days       = None if show_all else args.commit_days
    archive_days      = args.archive_days
    deep_archive_days = args.deep_archive_days
    dirty_only        = args.dirty    and not show_all
    unpushed_only     = args.unpushed and not show_all
    no_archive        = args.no_archive
    yes               = args.yes
    zip_dir           = Path(args.zip_dir) if args.zip_dir else None

    skipped = 0
    results = []

    for path, proj in sorted(projects.items(), key=lambda x: x[1]["name"].lower()):
        name          = proj["name"]
        missing       = not Path(path).exists()
        archived      = proj.get("archived", False)
        deep_archived = proj.get("deep_archived", False)

        lf_path, lf_time = (None, None) if missing else last_modified_file(path)
        gs = {} if missing else get_git_status(path)

        has_dirty    = bool(gs.get("staged") or gs.get("unstaged") or gs.get("untracked"))
        has_unpushed = bool(gs.get("unpushed"))
        nm_size      = None if missing else node_modules_size(path)

        # ── State transitions ────────────────────────────────────────────────
        #   active → archived      when last edit > archive_days
        #   archived → deep_archived when last edit > deep_archive_days (zip)

        should_archive = (
                not no_archive and not archived and not missing
                and lf_time is not None
                and not within_days(lf_time, archive_days)
        )

        should_deep_archive = (
                not no_archive and archived and not deep_archived and not missing
                and lf_time is not None
                and not within_days(lf_time, deep_archive_days)
                and not has_dirty and not has_unpushed  # only zip if fully clean
        )

        # ── Visibility filter ────────────────────────────────────────────────
        edited_ok    = within_days(lf_time,              edited_days)
        committed_ok = within_days(gs.get("last_commit"), commit_days)
        needs_show   = (missing or should_archive or should_deep_archive
                        or edited_ok or committed_ok or has_dirty or has_unpushed)

        if dirty_only or unpushed_only:
            needs_show = (dirty_only and has_dirty) or (unpushed_only and has_unpushed)

        if not needs_show and not show_all:
            skipped += 1
            continue

        # ── Handle: archive (active → archived) ──────────────────────────────
        nm_deleted   = False
        zip_path_str = None
        zip_error    = None

        if should_archive:
            db["projects"][path]["archived"]    = True
            db["projects"][path]["archived_at"] = datetime.now().isoformat(timespec="seconds")
            archived  = True
            db_dirty  = True

            if fmt != "json":
                age = time_ago(lf_time)
                print(f"\n  {BOLD}{CYAN}{name}{RESET}  {DIM}{path}{RESET}")
                print(f"  {'─' * 60}")
                print(f"  {BLUE}⬛ Marking as archived{RESET}  "
                      f"{DIM}(last edited {age}, over {archive_days}d threshold){RESET}")

                nm = Path(path) / "node_modules"
                if nm.exists():
                    size_str = fmt_size(nm_size) if nm_size else "?"
                    if yes:
                        shutil.rmtree(nm, ignore_errors=True)
                        nm_deleted = True
                        print(f"  {GREEN}✔{RESET}  Deleted node_modules ({size_str})")
                    else:
                        try:
                            ans = input(f"  {YELLOW}?{RESET}  Delete node_modules ({size_str})? [y/N] ").strip().lower()
                        except (EOFError, KeyboardInterrupt):
                            print(); ans = "n"
                        if ans == "y":
                            shutil.rmtree(nm, ignore_errors=True)
                            nm_deleted = True
                            print(f"  {GREEN}✔{RESET}  Deleted node_modules ({size_str})")
                        else:
                            print(f"  {DIM}  node_modules kept{RESET}")
                else:
                    print(f"  {DIM}  node_modules already absent{RESET}")
            else:
                nm = Path(path) / "node_modules"
                if nm.exists() and yes:
                    shutil.rmtree(nm, ignore_errors=True)
                    nm_deleted = True

        # ── Handle: deep archive (archived → deep_archived, zip) ─────────────
        elif should_deep_archive:
            if fmt != "json":
                age = time_ago(lf_time)
                print(f"\n  {BOLD}{CYAN}{name}{RESET}  {DIM}{path}{RESET}")
                print(f"  {'─' * 60}")
                print(f"  {MAGENTA}🗜  Deep archiving{RESET}  "
                      f"{DIM}(archived project, last edited {age}, over {deep_archive_days}d){RESET}")

                do_zip = yes
                if not yes:
                    print(f"\n  {DIM}This will:{RESET}")
                    print(f"  {DIM}  1. Zip the entire project folder{RESET}")
                    print(f"  {DIM}  2. Verify every file in the zip{RESET}")
                    print(f"  {DIM}  3. Ask before removing the original folder{RESET}")
                    try:
                        ans = input(f"\n  {YELLOW}?{RESET}  Proceed with deep archive? [y/N] ").strip().lower()
                    except (EOFError, KeyboardInterrupt):
                        print(); ans = "n"
                    do_zip = ans == "y"

                if do_zip:
                    print(f"  {DIM}  Zipping...{RESET}", end="", flush=True)
                    success, result = deep_archive_project(path, yes, zip_dir)
                    if success:
                        zip_size = Path(result).stat().st_size
                        print(f"\r  {GREEN}✔{RESET}  Zipped → {result}  {DIM}({fmt_size(zip_size)}){RESET}")
                        zip_path_str = result

                        # Ask to remove original
                        do_remove = yes
                        if not yes:
                            try:
                                ans = input(f"  {YELLOW}?{RESET}  Remove original folder? [y/N] ").strip().lower()
                            except (EOFError, KeyboardInterrupt):
                                print(); ans = "n"
                            do_remove = ans == "y"

                        if do_remove:
                            shutil.rmtree(path, ignore_errors=True)
                            print(f"  {GREEN}✔{RESET}  Original folder removed.")

                        db["projects"][path]["deep_archived"]    = True
                        db["projects"][path]["deep_archived_at"] = datetime.now().isoformat(timespec="seconds")
                        db["projects"][path]["zip_path"]         = result
                        deep_archived = True
                        db_dirty = True
                    else:
                        zip_error = result
                        print(f"\r  {RED}✖  Zip failed:{RESET} {result}")
                        print(f"  {DIM}  Original folder untouched.{RESET}")
                else:
                    print(f"  {DIM}  Skipped.{RESET}")
            else:
                # JSON / --yes mode
                if yes:
                    success, result = deep_archive_project(path, yes, zip_dir)
                    if success:
                        zip_path_str = result
                        shutil.rmtree(path, ignore_errors=True)
                        db["projects"][path]["deep_archived"]    = True
                        db["projects"][path]["deep_archived_at"] = datetime.now().isoformat(timespec="seconds")
                        db["projects"][path]["zip_path"]         = result
                        deep_archived = True
                        db_dirty = True
                    else:
                        zip_error = result

        # ── Build result dict ────────────────────────────────────────────────
        d = {
            "name":          name,
            "path":          path,
            "exists":        not missing,
            "archived":      archived,
            "archived_at":   db["projects"][path].get("archived_at") if path in db["projects"] else proj.get("archived_at"),
            "deep_archived": deep_archived,
            "deep_archived_at": db["projects"][path].get("deep_archived_at") if path in db["projects"] else proj.get("deep_archived_at"),
            "zip_path":      zip_path_str or proj.get("zip_path"),
            "zip_error":     zip_error,
            "stale":         should_archive,
            "last_file":     str(lf_path.relative_to(path)) if lf_path else None,
            "last_file_at":  lf_time.isoformat() if lf_time else None,
            "node_modules_size":    nm_size,
            "node_modules_deleted": nm_deleted,
            "has_dirty":     has_dirty,
            "has_unpushed":  has_unpushed,
            "git": {
                "is_repo":      gs.get("is_git", False),
                "branch":       gs.get("branch"),
                "last_commit":  gs["last_commit"].isoformat() if gs.get("last_commit") else None,
                "last_sha":     gs.get("last_sha"),
                "last_subject": gs.get("last_subject"),
                "last_author":  gs.get("last_author"),
                "unpushed":     gs.get("unpushed", []),
                "staged":       gs.get("staged", []),
                "unstaged":     gs.get("unstaged", []),
                "untracked":    gs.get("untracked", []),
            } if not missing else None,
        }
        results.append(d)

        # ── Normal print (non-transition projects) ───────────────────────────
        if fmt != "json" and not should_archive and not should_deep_archive:
            _print_header(name, path, archived, deep_archived)

            if missing:
                print(f"  {RED}✖  Project folder missing from disk{RESET}")
            else:
                if deep_archived:
                    print(f"  {MAGENTA}🗜  Deep archived{RESET}  "
                          f"{DIM}zip: {proj.get('zip_path', '?')}{RESET}")
                elif archived:
                    print(f"  {BLUE}⬛ Archived{RESET}  {DIM}since {proj.get('archived_at', '?')}{RESET}")

                if lf_time:
                    rel  = lf_path.relative_to(path)
                    flag = f"  {YELLOW}(>{archive_days}d){RESET}" if not within_days(lf_time, archive_days) else ""
                    print(f"\n  {BOLD}Last file edit{RESET}  {time_ago(lf_time)}  "
                          f"{DIM}({lf_time.strftime('%Y-%m-%d %H:%M')} UTC)  {rel}{RESET}{flag}")
                else:
                    print(f"  {DIM}No files found.{RESET}")

                if not gs.get("is_git"):
                    print(f"  {DIM}(not a git repository){RESET}")
                else:
                    print(f"  {BOLD}Branch{RESET}         {MAGENTA}{gs['branch']}{RESET}")
                    if gs["last_commit"]:
                        flag = (f"  {YELLOW}(>{commit_days}d){RESET}"
                                if commit_days and not within_days(gs["last_commit"], commit_days) else "")
                        print(f"  {BOLD}Last commit{RESET}    {time_ago(gs['last_commit'])}  "
                              f"{DIM}{gs['last_sha']}  {gs['last_subject']}  ({gs['last_author']}){RESET}{flag}")
                    else:
                        print(f"  {YELLOW}No commits yet{RESET}")

                    _print_group("Unpushed", YELLOW, gs["unpushed"])
                    _print_group("Staged",   GREEN,  gs["staged"])
                    _print_group("Unstaged", YELLOW, gs["unstaged"])
                    _print_group("Untracked", RED,   gs["untracked"])

                    if not has_dirty and not has_unpushed:
                        print(f"  {GREEN}✔{RESET}  Working tree clean")
            print()

    # ── Persist DB ───────────────────────────────────────────────────────────
    if db_dirty:
        save_db(db)

    # ── JSON output ──────────────────────────────────────────────────────────
    if fmt == "json":
        print(json.dumps({
            "projects": results,
            "skipped":  skipped,
            "summary": {
                "shown":         len(results),
                "skipped":       skipped,
                "dirty":         sum(1 for r in results if r["has_dirty"]),
                "unpushed":      sum(1 for r in results if r["has_unpushed"]),
                "archived":      sum(1 for r in results if r["archived"] and not r["deep_archived"]),
                "deep_archived": sum(1 for r in results if r["deep_archived"]),
                "stale":         sum(1 for r in results if r["stale"]),
            },
        }, indent=2))
        return

    if skipped:
        print(f"{DIM}  {skipped} project(s) skipped (outside thresholds). "
              f"Use --all to show everything.{RESET}")
    if not results:
        ok("All projects are outside the thresholds. Nothing to report.")
