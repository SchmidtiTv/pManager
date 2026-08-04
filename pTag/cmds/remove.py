import sys

from helper.db import load_db, resolve, save_db
from helper.logger import err, ok


def cmd_remove(args):
    db = load_db()
    path = resolve(args.path)

    if path not in db["projects"]:
        err(f"Project not tracked: {path}")
        sys.exit(1)

    name = db["projects"][path]["name"]
    del db["projects"][path]
    save_db(db)
    ok(f"Removed '{name}' from database.")
