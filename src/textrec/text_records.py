"""
Text Records Management
Read, write, and join text files broken into records with ---- separators.
Reads 3+ hyphens for backward compatibility, writes 4 hyphens (asciidoc standard).
"""

import logging
import os
import re
import urllib.parse
from datetime import datetime
from pathlib import Path
from typing import Iterable, List

RECORD_SEPARATOR = "----"
_SEPARATOR_RE = re.compile(r'^-{3,}\s*$', re.MULTILINE)

logger = logging.getLogger(__name__)

class TextRecords:
    """Reads, writes, and formats text files broken into records with ---- separators."""

    def __init__(self, path: Path):
        # Cursor encodes : as %3A in file paths; decode if present.
        path_str = str(path)
        if '%' in path_str:
            path_str = urllib.parse.unquote(path_str)
        self.path = Path(path_str)

    @staticmethod
    def read_records(file_path: Path) -> List[str]:
        """Read a file and return its records (separated by 3+ hyphens on a line) as plain strings."""
        if not file_path.exists():
            logger.info(f"File does not exist: {file_path}, returning empty list")
            return []
        try:
            with open(file_path, "r", encoding="utf-8") as f:
                content = f.read()
            return [part.strip() for part in _SEPARATOR_RE.split(content) if part.strip()]
        except Exception as e:
            logger.error(f"Error reading file {file_path}: {e}")
            return []
    
    @staticmethod
    def join_records(record_texts: Iterable[str]) -> str:
        """Join already-formatted record texts with the canonical record separator."""
        return f"\n{RECORD_SEPARATOR}\n".join(record_texts)

    def write_atomic(self, file_path: Path, content: str) -> None:
        """Atomically write content, archiving the previous file into bak/ if one exists."""
        try:
            self._archive(file_path)
            self._atomic_replace(file_path, content)
        except Exception as exc:
            logger.error("Error writing file %s: %s", file_path, exc)
            raise

    def write_atomic_no_backup(self, file_path: Path, content: str) -> None:
        """Atomically write content, replacing the previous file without keeping a copy."""
        try:
            self._atomic_replace(file_path, content)
        except Exception as exc:
            logger.error("Error writing file %s: %s", file_path, exc)
            raise

    @staticmethod
    def _archive(file_path: Path) -> None:
        """Move an existing file into a sibling bak/ directory with a timestamped name."""
        if not file_path.exists():
            return
        timestamp = datetime.now().strftime("%Y%m%d%H%M%S")
        bak_dir = file_path.parent / "bak"
        bak_dir.mkdir(parents=True, exist_ok=True)
        bak_path = bak_dir / f"{file_path.stem}.{timestamp}{file_path.suffix}"
        os.replace(file_path, bak_path)

    @staticmethod
    def _atomic_replace(file_path: Path, content: str) -> None:
        """Write content to a temp sibling and atomically swap it into place."""
        file_path.parent.mkdir(parents=True, exist_ok=True)
        timestamp = datetime.now().strftime("%Y%m%d%H%M%S")
        new_path = file_path.with_name(f"{file_path.stem}.{timestamp}{file_path.suffix}")
        with open(new_path, "w", encoding="utf-8") as fh:
            fh.write(content)
        os.replace(new_path, file_path) 