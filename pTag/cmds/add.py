import json
import sys
from datetime import datetime
from pathlib import Path

from helper.colors import RESET, BOLD
from helper.db import load_db, resolve, save_db
from helper.logger import err, ok, warn
from helper.project import is_node_project


def cmd_add(args):
    db   = load_db()
    path = resolve(args.path)
    p    = Path(path)

    if not p.exists():
        err(f"Path does not exist: {path}")
        sys.exit(1)

    if not is_node_project(path):
        warn("No package.json found — are you sure this is a Node project? Adding anyway.")

    if path in db["projects"]:
        warn("Project already tracked.")
        db["projects"][path]["updated"] = datetime.now().isoformat(timespec="seconds")
        save_db(db)
        return

    name = p.name
    try:
        with open(p / "package.json", encoding="utf-8") as f:
            name = json.load(f).get("name", p.name)
    except Exception:
        pass

    db["projects"][path] = {
        "name":     name,
        "path":     path,
        "archived": False,
        "added":    datetime.now().isoformat(timespec="seconds"),
        "updated":  datetime.now().isoformat(timespec="seconds"),
    }
    save_db(db)
    ok(f"Added '{BOLD}{name}{RESET}' ({path})")
