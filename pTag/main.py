#!/usr/bin/env python3
import argparse

from cmds.add import cmd_add
from cmds.check import cmd_check
from cmds.info import cmd_info
from cmds.list import cmd_list
from cmds.remove import cmd_remove
from cmds.restore import cmd_restore
from cmds.sync import cmd_sync
from constants import DEFAULT_ARCHIVE_DAYS, DEFAULT_EDITED_DAYS, DEFAULT_COMMIT_DAYS


def _add_format(p):
    p.add_argument(
        "--format", "-f",
        choices=["normal", "json"],
        default="normal",
        metavar="FMT",
        help="Output format: normal (default) or json",
    )


def main():
    parser = argparse.ArgumentParser(
        prog="ptag",
        description="Track and manage Node.js projects from the command line.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    sub = parser.add_subparsers(dest="command", metavar="<command>")
    sub.required = True

    # add
    p_add = sub.add_parser("add", help="Add a project to the database")
    p_add.add_argument("path", help="Path to the Node.js project")
    p_add.set_defaults(func=cmd_add)

    # remove
    p_rm = sub.add_parser("remove", aliases=["rm"], help="Remove a project from the database")
    p_rm.add_argument("path", help="Path to the project")
    p_rm.set_defaults(func=cmd_remove)

    # list
    p_ls = sub.add_parser("list", aliases=["ls"], help="List tracked projects")
    _add_format(p_ls)
    p_ls.set_defaults(func=cmd_list)

    # info
    p_info = sub.add_parser("info", help="Show details about a tracked project")
    p_info.add_argument("path", help="Path to the project")
    _add_format(p_info)
    p_info.set_defaults(func=cmd_info)

    # sync
    p_sync = sub.add_parser("sync", help="Check project status and remove missing entries")
    p_sync.add_argument(
        "--archive-days", "-A", type=int, default=DEFAULT_ARCHIVE_DAYS, metavar="DAYS",
        help=f"Threshold for flagging a project as stale (default: {DEFAULT_ARCHIVE_DAYS})",
    )
    _add_format(p_sync)
    p_sync.set_defaults(func=cmd_sync)

    # check
    p_check = sub.add_parser(
        "check",
        help="Inspect git status and file changes; auto-archive stale projects",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        description=(
            "Shows projects active within thresholds or with uncommitted/unpushed changes.\n"
            "Projects last edited beyond --archive-days are marked archived and prompted\n"
            "to have their node_modules deleted."
        ),
    )
    p_check.add_argument("path", nargs="?", default=None, help="Check a single project")
    p_check.add_argument("--all", "-a", action="store_true", help="Show all projects")
    p_check.add_argument("--edited-days", "-e", type=int, default=DEFAULT_EDITED_DAYS, metavar="DAYS",
                         help=f"File-edit recency threshold (default: {DEFAULT_EDITED_DAYS})")
    p_check.add_argument("--commit-days", "-c", type=int, default=DEFAULT_COMMIT_DAYS, metavar="DAYS",
                         help=f"Commit recency threshold (default: {DEFAULT_COMMIT_DAYS})")
    p_check.add_argument("--archive-days", "-A", type=int, default=DEFAULT_ARCHIVE_DAYS, metavar="DAYS",
                         help=f"Archive threshold in days (default: {DEFAULT_ARCHIVE_DAYS})")
    p_check.add_argument("--no-archive", action="store_true", help="Disable automatic archiving")
    p_check.add_argument("--dirty", "-d", action="store_true", help="Only projects with uncommitted changes")
    p_check.add_argument("--unpushed", "-u", action="store_true", help="Only projects with unpushed commits")
    p_check.add_argument("--yes", "-y", action="store_true", help="Auto-confirm node_modules deletion")
    p_check.add_argument("--deep-archive-days", "-D", type=int, default=180, metavar="DAYS",
                         help="Deep archive threshold in days (default: 180)")
    p_check.add_argument("--zip-dir", type=str, default=None, metavar="PATH",
                         help="Where to store .zip files (default: same folder as project)")

    _add_format(p_check)
    p_check.set_defaults(func=cmd_check)

    # restore
    p_restore = sub.add_parser(
        "restore",
        help="Restore a deep-archived project (unzip + npm install)",
    )
    p_restore.add_argument("path", help="Tracked path or project name")
    p_restore.add_argument(
        "--restore-dir", "-r", metavar="DIR", default=None,
        help="Extract into this directory instead of the original parent",
    )
    p_restore.add_argument(
        "--yes", "-y", action="store_true",
        help="Auto-confirm all prompts",
    )
    p_restore.add_argument(
        "--keep-zip", "-k", action="store_true",
        help="Keep the zip file after successful restore",
    )
    p_restore.add_argument(
        "--verbose", "-v", action="store_true",
        help="Show full npm install output",
    )
    p_restore.set_defaults(func=cmd_restore)

    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
