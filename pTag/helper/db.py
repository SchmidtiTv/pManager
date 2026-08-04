import json
from pathlib import Path

from constants import DB_PATH


def load_db() -> dict:
    if DB_PATH.exists():
        with open(DB_PATH, "r", encoding="utf-8") as f:
            return json.load(f)
    return {"projects": {}}


def save_db(db: dict) -> None:
    DB_PATH.parent.mkdir(parents=True, exist_ok=True)
    with open(DB_PATH, "w", encoding="utf-8") as f:
        json.dump(db, f, indent=2)


def resolve(path_str: str) -> str:
    return str(Path(path_str).expanduser().resolve())
