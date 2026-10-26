"""Compatibility package exposing the canonical scripts.src constants."""

from pathlib import Path as _Path
import sys as _sys

_legacy_name = __name__
_repo_root = _Path(__file__).resolve().parents[2]
_repo_root_text = str(_repo_root)
if _repo_root_text not in _sys.path:
    _sys.path.insert(0, _repo_root_text)
import scripts.src as _canonical

globals().update(vars(_canonical))
_sys.modules[_legacy_name] = _canonical
