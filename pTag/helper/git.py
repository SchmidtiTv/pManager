# ── Git helpers ────────────────────────────────────────────────────────────────
import subprocess
from datetime import datetime


def git(args: list, cwd: str) -> tuple[int, str]:
    try:
        r = subprocess.run(["git"] + args, cwd=cwd, capture_output=True, text=True)
        return r.returncode, r.stdout.strip()
    except FileNotFoundError:
        return -1, ""


def is_git_repo(path: str) -> bool:
    code, _ = git(["rev-parse", "--git-dir"], path)
    return code == 0

def get_git_status(path: str) -> dict:
    result = {
        "is_git": False, "branch": None,
        "last_commit": None, "last_sha": None,
        "last_subject": None, "last_author": None,
        "unpushed": [], "staged": [], "unstaged": [], "untracked": [],
    }
    if not is_git_repo(path):
        return result
    result["is_git"] = True
    _, branch = git(["rev-parse", "--abbrev-ref", "HEAD"], path)
    result["branch"] = branch
    _, log = git(["log", "-1", "--format=%h|%s|%ci|%an"], path)
    if log:
        parts = log.split("|", 3)
        if len(parts) == 4:
            sha, subject, date_str, author = parts
            try:
                result["last_commit"]  = datetime.fromisoformat(date_str.strip())
                result["last_sha"]     = sha
                result["last_subject"] = subject
                result["last_author"]  = author
            except ValueError:
                pass
    _, unpushed = git(["log", "@{u}..", "--oneline"], path)
    result["unpushed"] = [l for l in unpushed.splitlines() if l.strip()]
    _, status_out = git(["status", "--porcelain"], path)
    for line in status_out.splitlines():
        if not line.strip():
            continue
        if line.startswith("??"):
            result["untracked"].append(line[3:])
        else:
            if line[0] in "MADRC" and line[0] != " ":
                result["staged"].append(line[3:])
            if line[1] in "MD" and line[1] != " ":
                result["unstaged"].append(line[3:])
    return result
