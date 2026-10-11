from pathlib import Path as _Path
import importlib as _importlib
import sys as _sys

# Preserve the historical import path while resolving implementation from the
# canonical package, even when launched by absolute path from another cwd.
_legacy_name = __name__
_repo_root = _Path(__file__).resolve().parents[2]
_repo_root_text = str(_repo_root)
if _repo_root_text not in _sys.path:
    _sys.path.insert(0, _repo_root_text)
_canonical = _importlib.import_module("scripts.src.signal_flow")
globals().update(vars(_canonical))
_sys.modules[_legacy_name] = _canonical
