import os
from pathlib import Path

# ── Database location ──────────────────────────────────────────────────────────
DB_PATH = Path(os.environ.get("PTAG_DB", Path.home() / ".ptag" / "db.json"))

# ── Default thresholds ─────────────────────────────────────────────────────────
DEFAULT_EDITED_DAYS  = 60
DEFAULT_COMMIT_DAYS  = 60
DEFAULT_ARCHIVE_DAYS = 60