import shutil
import subprocess
import sys
import zipfile
from datetime import datetime
from pathlib import Path

from helper.colors import RESET, BOLD, GREEN, YELLOW, RED, CYAN, DIM, MAGENTA, BLUE
from helper.db import load_db, resolve, save_db
from helper.logger import err, info, ok, warn
from helper.project import fmt_size


# ── Helpers ────────────────────────────────────────────────────────────────────

def _unzip_project(zip_path: Path, dest_parent: Path) -> tuple[bool, str]:
    """
    Safely extract zip_path into dest_parent.
      1. Verify zip integrity first (testzip) before touching the filesystem
      2. Extract into a temp sibling directory
      3. Atomic rename to final destination on success
      4. Rollback (rm temp dir) on any failure

    Returns (success, extracted_path_or_error_message).
    """
    tmp_dir = dest_parent / f".ptag_restore_tmp_{zip_path.stem}"
    if tmp_dir.exists():
        shutil.rmtree(tmp_dir)

    try:
        with zipfile.ZipFile(zip_path, "r") as zf:
            # Step 1: integrity check before touching disk
            bad = zf.testzip()
            if bad:
                return False, f"Zip integrity check failed on entry: {bad}"

            # Step 2: extract to temp dir
            zf.extractall(tmp_dir)

        # Step 3: verify something was actually written
        extracted_files = list(tmp_dir.rglob("*"))
        if not extracted_files:
            shutil.rmtree(tmp_dir, ignore_errors=True)
            return False, "Extraction produced no files."

        # Step 4: determine final folder name (top-level dir inside zip)
        top_level = list(tmp_dir.iterdir())
        if len(top_level) == 1 and top_level[0].is_dir():
            final_dir = dest_parent / top_level[0].name
        else:
            final_dir = dest_parent / zip_path.stem

        if final_dir.exists():
            shutil.rmtree(tmp_dir, ignore_errors=True)
            return False, (
                f"Destination already exists: {final_dir}\n"
                f"       Remove it first or use --restore-dir to choose a different location."
            )

        # Step 5: atomic rename
        if len(top_level) == 1 and top_level[0].is_dir():
            top_level[0].rename(final_dir)
            shutil.rmtree(tmp_dir, ignore_errors=True)
        else:
            tmp_dir.rename(final_dir)

        return True, str(final_dir)

    except Exception as ex:
        shutil.rmtree(tmp_dir, ignore_errors=True)
        return False, str(ex)


def _run_npm_install(project_path: Path, yes: bool, verbose: bool = False) -> tuple[bool, str]:
    """
    Run `npm install` inside project_path.
    Prompts unless yes=True.
    Returns (ran, message).
    """
    pkg = project_path / "package.json"
    if not pkg.exists():
        return False, "No package.json found — skipping npm install."

    if not yes:
        try:
            ans = input(
                f"  {YELLOW}?{RESET}  Run {BOLD}npm install{RESET} in "
                f"{BOLD}{project_path.name}{RESET}? [Y/n] "
            ).strip().lower()
        except (EOFError, KeyboardInterrupt):
            print()
            return False, "Skipped (interrupted)."
        if ans == "n":
            return False, "Skipped by user."

    print(f"  {DIM}  Running npm install...{RESET}", flush=True)
    try:
        result = subprocess.run(
            ["npm", "install"],
            cwd=str(project_path),
            text=True,
            capture_output=not verbose,
        )
        if result.returncode == 0:
            out = result.stdout.strip() if result.stdout else ""
            return True, out
        else:
            out = (result.stderr or result.stdout or "").strip()
            return False, out
    except FileNotFoundError:
        return False, "`npm` not found in PATH. Install Node.js and try again."


# ── Command ────────────────────────────────────────────────────────────────────

def cmd_restore(args):
    db       = load_db()
    projects = db["projects"]
    path_key = resolve(args.path)

    # ── 1. Look up the project (by path or by name) ──────────────────────────
    proj = projects.get(path_key)
    if proj is None:
        matches = [
            (k, v) for k, v in projects.items()
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
    deep_archived = proj.get("deep_archived", False)
    archived      = proj.get("archived", False)

    # ── 2. Guard: must be deep-archived ─────────────────────────────────────
    if not deep_archived:
        if archived:
            err(
                f"'{name}' is archived (not deep-archived). "
                "The project folder should still be on disk — no zip to extract."
            )
        else:
            err(f"'{name}' is active. Nothing to restore.")
        sys.exit(1)

    zip_path_str = proj.get("zip_path")
    if not zip_path_str:
        err(f"No zip_path recorded for '{name}'. Cannot restore.")
        sys.exit(1)

    zip_path = Path(zip_path_str)
    if not zip_path.exists():
        err(f"Zip file not found: {zip_path}")
        sys.exit(1)

    # ── 3. Restore destination ───────────────────────────────────────────────
    if args.restore_dir:
        restore_parent = Path(args.restore_dir).expanduser().resolve()
    else:
        restore_parent = Path(path_key).parent

    restore_parent.mkdir(parents=True, exist_ok=True)
    zip_size = zip_path.stat().st_size

    # ── 4. Print summary ─────────────────────────────────────────────────────
    print(f"\n  {BOLD}{CYAN}{name}{RESET}  {MAGENTA}[deep-archived]{RESET}  {DIM}{path_key}{RESET}")
    print(f"  {'-' * 60}")
    print(f"  {MAGENTA}Zip {RESET}  {DIM}{zip_path}{RESET}  {DIM}({fmt_size(zip_size)}){RESET}")
    print(f"  {CYAN}Into{RESET}  {DIM}{restore_parent}{RESET}")
    print()

    # ── 5. Confirm ───────────────────────────────────────────────────────────
    if not args.yes:
        try:
            ans = input(f"  {YELLOW}?{RESET}  Extract and restore project? [Y/n] ").strip().lower()
        except (EOFError, KeyboardInterrupt):
            print()
            info("Aborted.")
            return
        if ans == "n":
            info("Aborted.")
            return

    # ── 6. Extract ───────────────────────────────────────────────────────────
    print(f"  {DIM}  Extracting...{RESET}", end="", flush=True)
    success, result = _unzip_project(zip_path, restore_parent)

    if not success:
        print()
        err(f"Extraction failed: {result}")
        sys.exit(1)

    restored_path = Path(result)
    print(f"\r  {GREEN}OK{RESET}  Extracted  {DIM}{restored_path}{RESET}")

    # ── 7. npm install ───────────────────────────────────────────────────────
    ran_npm, npm_msg = _run_npm_install(restored_path, args.yes, verbose=args.verbose)
    if ran_npm:
        print(f"  {GREEN}OK{RESET}  npm install complete")
    else:
        warn(f"npm install: {npm_msg}")

    # ── 8. Update DB ─────────────────────────────────────────────────────────
    new_path_key = str(restored_path)
    restored_at  = datetime.now().isoformat(timespec="seconds")

    if new_path_key == path_key:
        # Restored to original location — update in place
        proj_entry = db["projects"][path_key]
        proj_entry["deep_archived"]    = False
        proj_entry["archived"]         = False
        proj_entry["deep_archived_at"] = None
        proj_entry["archived_at"]      = None
        proj_entry["zip_path"]         = None
        proj_entry["restored_at"]      = restored_at
    else:
        # Restored to a different directory — register under new path
        entry = dict(proj)
        entry["path"]             = new_path_key
        entry["deep_archived"]    = False
        entry["archived"]         = False
        entry["deep_archived_at"] = None
        entry["archived_at"]      = None
        entry["zip_path"]         = None
        entry["restored_at"]      = restored_at
        db["projects"][new_path_key] = entry
        db["projects"][path_key]["restored_to"] = new_path_key

    save_db(db)

    print()
    ok(f"'{BOLD}{name}{RESET}' restored and marked active in the database.")

    # ── 9. Offer to delete the zip ───────────────────────────────────────────
    if not args.keep_zip:
        do_delete = args.yes
        if not args.yes:
            try:
                ans = input(
                    f"\n  {YELLOW}?{RESET}  Delete the zip now?  "
                    f"{DIM}({fmt_size(zip_size)}){RESET} [y/N] "
                ).strip().lower()
            except (EOFError, KeyboardInterrupt):
                print()
                ans = "n"
            do_delete = (ans == "y")

        if do_delete:
            zip_path.unlink(missing_ok=True)
            ok(f"Deleted zip: {zip_path}")
        else:
            info(f"Zip kept: {zip_path}")

    print()
