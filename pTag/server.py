"""
ptag web UI  —  Flask server
Run:  python server.py
      python server.py --port 5050
      python server.py --db path/to/db.json
"""
import argparse
import json
import os
from datetime import datetime, timezone, timedelta
from pathlib import Path

from flask import Flask, jsonify, render_template, abort, request

app = Flask(__name__)

# ── Config (set before first request) ─────────────────────────────────────────
DB_PATH: Path = Path(os.environ.get("PTAG_DB", Path.home() / ".flsc/.ptag/db.json"))


# ── Helpers ───────────────────────────────────────────────────────────────────

def load_db() -> dict:
    if DB_PATH.exists():
        with open(DB_PATH, "r", encoding="utf-8") as f:
            return json.load(f)
    return {"projects": {}}


def time_ago(iso: str | None) -> str | None:
    if not iso:
        return None
    try:
        dt = datetime.fromisoformat(iso)
    except ValueError:
        return iso
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    now = datetime.now(timezone.utc)
    s = int((now - dt).total_seconds())
    if s < 0:       return "just now"
    if s < 60:      return f"{s}s ago"
    if s < 3600:    return f"{s // 60}m ago"
    if s < 86400:   return f"{s // 3600}h ago"
    if s < 86400*30:  return f"{s // 86400}d ago"
    if s < 86400*365: return f"{s // (86400 * 30)}mo ago"
    return f"{s // (86400 * 365)}y ago"


def fmt_size(n: int | None) -> str | None:
    if n is None:
        return None
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024:
            return f"{n:.1f} {unit}"
        n /= 1024
    return f"{n:.1f} TB"


def project_state(proj: dict, path: str) -> str:
    if proj.get("deep_archived"):
        return "deep_archived"
    if proj.get("archived"):
        return "archived"
    if not Path(path).exists():
        return "missing"
    return "active"


def enrich(path: str, proj: dict) -> dict:
    """Return a serialisable dict ready for the API."""
    state    = project_state(proj, path)
    exists   = Path(path).exists()

    zip_path = proj.get("zip_path")
    zip_size = None
    if zip_path:
        zp = Path(zip_path)
        if zp.exists():
            try:
                zip_size = zp.stat().st_size
            except OSError:
                pass

    # Derive has_dirty / has_unpushed from stored last-check snapshot if available
    # (We don't rescan git on every page load for performance)
    return {
        "name":             proj["name"],
        "path":             path,
        "state":            state,
        "exists":           exists,
        "archived":         proj.get("archived", False),
        "deep_archived":    proj.get("deep_archived", False),
        "archived_at":      proj.get("archived_at"),
        "archived_at_ago":  time_ago(proj.get("archived_at")),
        "deep_archived_at": proj.get("deep_archived_at"),
        "deep_archived_at_ago": time_ago(proj.get("deep_archived_at")),
        "zip_path":         zip_path,
        "zip_size":         zip_size,
        "zip_size_fmt":     fmt_size(zip_size),
        "added":            proj.get("added"),
        "added_ago":        time_ago(proj.get("added")),
        "updated":          proj.get("updated"),
        "updated_ago":      time_ago(proj.get("updated")),
        "restored_at":      proj.get("restored_at"),
        "restored_to":      proj.get("restored_to"),
        # Cached from last `ptag check` run if stored
        "last_file_at":     proj.get("last_file_at"),
        "last_file_at_ago": time_ago(proj.get("last_file_at")),
        "git": {
            "branch":       proj.get("git_branch"),
            "last_sha":     proj.get("git_last_sha"),
            "last_subject": proj.get("git_last_subject"),
            "last_commit":  proj.get("git_last_commit"),
            "last_commit_ago": time_ago(proj.get("git_last_commit")),
            "has_dirty":    proj.get("has_dirty", False),
            "has_unpushed": proj.get("has_unpushed", False),
            "staged":       proj.get("staged", []),
            "unstaged":     proj.get("unstaged", []),
            "untracked":    proj.get("untracked", []),
            "unpushed":     proj.get("unpushed", []),
        },
    }


# ── Routes ────────────────────────────────────────────────────────────────────

@app.route("/")
def index():
    return render_template("index.html")


@app.route("/api/projects")
def api_projects():
    db       = load_db()
    projects = db.get("projects", {})
    result   = [enrich(path, proj) for path, proj in sorted(
        projects.items(), key=lambda x: x[1]["name"].lower()
    )]
    counts = {
        "total":         len(result),
        "active":        sum(1 for r in result if r["state"] == "active" and not r["git"]["has_dirty"] and not r["git"]["has_unpushed"]),
        "needs_attention": sum(1 for r in result if r["state"] == "active" and (r["git"]["has_dirty"] or r["git"]["has_unpushed"])),
        "archived":      sum(1 for r in result if r["state"] == "archived"),
        "deep_archived": sum(1 for r in result if r["state"] == "deep_archived"),
        "missing":       sum(1 for r in result if r["state"] == "missing"),
    }
    return jsonify({"projects": result, "counts": counts, "db_path": str(DB_PATH)})


@app.route("/api/projects/<path:proj_path>")
def api_project_detail(proj_path):
    db   = load_db()
    proj = db["projects"].get("/" + proj_path) or db["projects"].get(proj_path)
    if proj is None:
        # Windows-style path
        proj = db["projects"].get(proj_path.replace("/", "\\"))
    if proj is None:
        abort(404)
    return jsonify(enrich(proj_path, proj))



@app.route("/api/action", methods=["POST"])
def api_action():
    """Run a ptag CLI action on a project."""
    import subprocess, shutil
    data   = request.get_json(force=True)
    action = data.get("action", "")
    path   = data.get("path", "")

    if not action or not path:
        return jsonify({"ok": False, "output": "Missing action or path."})

    # Locate ptag/projtag script next to server.py
    prefix = ["ptag"]

    action_map = {
        "check":        prefix + ["check",   path],
        "archive":      prefix + ["archive", path],
        "unarchive":    prefix + ["unarchive", path],
        "deep_archive": prefix + ["deep-archive", path],
        "restore":      prefix + ["restore", path],
        "remove":       prefix + ["remove",  path],
        "open":         None,  # handled separately
    }

    if action not in action_map:
        return jsonify({"ok": False, "output": f"Unknown action: {action}"})

    if action == "open":
        import platform, os
        if not Path(path).exists():
            return jsonify({"ok": False, "output": "Path does not exist on disk."})
        opener = {"Darwin": "open", "Windows": "explorer", "Linux": "xdg-open"}.get(platform.system())
        if not opener:
            return jsonify({"ok": False, "output": "Unsupported OS for open action."})
        subprocess.Popen([opener, path])
        return jsonify({"ok": True, "output": f"Opened {path}"})

    cmd = action_map[action]
    try:
        print(f"Running: {' '.join(cmd)}")
        result = subprocess.run(
            cmd, capture_output=True, text=True, timeout=30,
            env={**__import__("os").environ, "PTAG_DB": str(DB_PATH)},
        )
        combined = (result.stdout + result.stderr).strip()
        return jsonify({"ok": result.returncode == 0, "output": combined or "Done."})
    except subprocess.TimeoutExpired:
        return jsonify({"ok": False, "output": "Command timed out."})
    except Exception as e:
        return jsonify({"ok": False, "output": str(e)})

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="ptag web UI")
    parser.add_argument("--port", "-p", type=int, default=5000)
    parser.add_argument("--host",       default="127.0.0.1")
    parser.add_argument("--db",         default=None, help="Path to ptag db.json")
    parser.add_argument("--debug",      action="store_true")
    a = parser.parse_args()

    if a.db:
        DB_PATH = Path(a.db).expanduser().resolve()

    print(f"  ptag UI  →  http://{a.host}:{a.port}")
    print(f"  DB       →  {DB_PATH}")
    app.run(host=a.host, port=a.port, debug=a.debug)
