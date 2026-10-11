#!/usr/bin/env python3
"""Generate src/settings.rs from qutebrowser's configdata.yml (settings schema).

Usage: scripts/gen-settings.py ../qutebrowser/qutebrowser/config/configdata.yml > src/settings.rs
Types are collapsed to JSON kinds; config.rs validates against them.
"""
import json
import sys

import yaml

KIND = {
    "Bool": "bool", "BoolAsk": "boolask", "Int": "int", "Float": "num",
    "List": "list", "FlagList": "list", "ShellCommand": "list", "ConfirmQuit": "list",
    "ListOrValue": "listorvalue", "Dict": "dict", "Padding": "dict",
    "Perc": "perc", "PercOrInt": "perc",
}
# Option types whose values live in configtypes.py, not in the yml.
VALUES = {
    "Position": ["top", "bottom", "left", "right"],
    "VerticalPosition": ["top", "bottom"],
    "TextAlignment": ["left", "right", "center"],
    "ElidePosition": ["left", "right", "middle", "none"],
    "NewTabPosition": ["prev", "next", "first", "last"],
    "SelectOnRemove": ["prev", "next", "last-used"],
    "IgnoreCase": ["always", "never", "smart"],
    "ColorSystem": ["rgb", "hsv", "hsl", "none"],
    "JSClipboardPermission": ["none", "access", "access-paste"],
}

def rs(s):
    return json.dumps(s, ensure_ascii=False)

with open(sys.argv[1]) as f:  # dev script: a bad path should just crash
    data = yaml.safe_load(f)
print("//! Settings schema generated from qutebrowser `configdata.yml` by")
print("//! `scripts/gen-settings.py`. Regenerate rather than hand-edit.")
print("//! (name, kind, none_ok, default as JSON, allowed string values)")
print()
print("pub const SETTINGS: &[(&str, &str, bool, &str, &[&str])] = &[")
for name, v in data.items():
    if not isinstance(v, dict) or "type" not in v:
        continue  # renamed/removed settings
    t = v["type"]
    tname = t if isinstance(t, str) else t["name"]
    none_ok = isinstance(t, dict) and bool(t.get("none_ok"))
    kind = KIND.get(tname, "str")
    values = VALUES.get(tname, []) if kind == "str" else []
    if kind == "str" and isinstance(t, dict) and "valid_values" in t:
        values = [next(iter(x)) if isinstance(x, dict) else x for x in t["valid_values"]]
    values = [str(x) for x in values]
    default = json.dumps(v.get("default"), ensure_ascii=False)
    vals = ", ".join(rs(x) for x in values)
    print(f"    ({rs(name)}, {rs(kind)}, {str(none_ok).lower()}, {rs(default)}, &[{vals}]),")
print("];")
