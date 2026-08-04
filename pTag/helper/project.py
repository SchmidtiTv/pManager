import os
from datetime import datetime, timezone, timedelta
from pathlib import Path


def is_node_project(path: str) -> bool:
    return (Path(path) / "package.json").exists()


def time_ago(dt: datetime) -> str:
    now = datetime.now(timezone.utc)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    s = int((now - dt).total_seconds())
    if s < 60:        return f"{s}s ago"
    if s < 3600:      return f"{s // 60}m ago"
    if s < 86400:     return f"{s // 3600}h ago"
    if s < 86400 * 30:  return f"{s // 86400}d ago"
    if s < 86400 * 365: return f"{s // (86400 * 30)}mo ago"
    return f"{s // (86400 * 365)}y ago"


def within_days(dt: datetime | None, days: int | None) -> bool:
    if days is None or dt is None:
        return True
    now = datetime.now(timezone.utc)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return (now - dt) <= timedelta(days=days)


def last_modified_file(path: str) -> tuple:
    SKIP = {"node_modules", ".git", ".next", "dist", "build", ".cache", "coverage"}
    latest_path, latest_time = None, None
    for root, dirs, files in os.walk(path):
        dirs[:] = [d for d in dirs if d not in SKIP]
        for fname in files:
            fpath = Path(root) / fname
            try:
                dt = datetime.fromtimestamp(fpath.stat().st_mtime, tz=timezone.utc)
                if latest_time is None or dt > latest_time:
                    latest_time, latest_path = dt, fpath
            except OSError:
                pass
    return latest_path, latest_time


def node_modules_size(path: str) -> int | None:
    nm = Path(path) / "node_modules"
    if not nm.exists():
        return None
    total = 0
    for f in nm.rglob("*"):
        try:
            if f.is_file():
                total += f.stat().st_size
        except OSError:
            pass
    return total


def fmt_size(n: int) -> str:
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024:
            return f"{n:.1f} {unit}"
        n /= 1024
    return f"{n:.1f} TB"
