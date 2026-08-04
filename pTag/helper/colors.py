import re

RESET   = "\033[0m"
BOLD    = "\033[1m"
GREEN   = "\033[32m"
YELLOW  = "\033[33m"
RED     = "\033[31m"
CYAN    = "\033[36m"
DIM     = "\033[2m"
MAGENTA = "\033[35m"
BLUE    = "\033[34m"

_ANSI_RE = re.compile(r"\033\[[0-9;]*m")