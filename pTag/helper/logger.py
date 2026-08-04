# ── ANSI helpers ───────────────────────────────────────────────────────────────
import sys

from helper.colors import RESET, GREEN, YELLOW, RED, CYAN, _ANSI_RE


def _vis(s: str) -> int:
    return len(_ANSI_RE.sub("", s))


def ok(msg):   print(f"{GREEN}[OK]{RESET}  {msg}")


def warn(msg): print(f"{YELLOW}⚠{RESET}  {msg}")


def err(msg):  print(f"{RED}✖{RESET}  {msg}", file=sys.stderr)


def info(msg): print(f"{CYAN}→{RESET}  {msg}")
