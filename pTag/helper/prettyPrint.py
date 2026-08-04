from helper.colors import RESET, BOLD, CYAN, DIM, BLUE


def _print_header(name: str, path: str, archived: bool = False, deep_archived: bool = False):
    from helper.colors import RESET, BOLD, CYAN, DIM, BLUE, MAGENTA

    if deep_archived:
        state_badge = f"  {MAGENTA}[deep-archived]{RESET}"
    elif archived:
        state_badge = f"  {BLUE}[archived]{RESET}"
    else:
        state_badge = ""

    print(f"\n  {BOLD}{CYAN}{name}{RESET}{state_badge}  {DIM}{path}{RESET}")
    print(f"  {'─' * 60}")



def _print_group(label: str, color: str, lines: list, limit: int = 10):
    if not lines:
        return
    print(f"  {BOLD}{color}{label}{RESET}  ({len(lines)})")
    for line in lines[:limit]:
        print(f"       {color}•{RESET}  {DIM}{line}{RESET}")
    if len(lines) > limit:
        print(f"       {DIM}… and {len(lines) - limit} more{RESET}")
