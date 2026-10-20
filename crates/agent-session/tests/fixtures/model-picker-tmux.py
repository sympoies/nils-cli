#!/usr/bin/env python3
"""Synthetic picker semantics: navigation, session-only s, and saving Enter."""
import json
import pathlib
import sys

root = pathlib.Path(__file__).parent
args = sys.argv[1:]
# Match tmux's argv command splitter: a bare semicolon is not a key.
assert not any(arg == ";" for arg in args), "unescaped tmux command separator"
with (root / "calls").open("a") as calls:
    calls.write(json.dumps(args) + "\n")
command = args[0]
if command == "has-session":
    sys.exit(0)
if command == "show-environment":
    values = {
        "AGENT_SESSION_ID": "picker",
        "AGENT_SESSION_RUNTIME_ID": "picker-runtime",
        "AGENT_SESSION_STATE_DIR": str(root / "state"),
    }
    key = args[-1]
    print(key + "=" + values[key])
elif command == "capture-pane":
    live = json.loads((root / "live.json").read_text())
    print("model: " + live["model"] + "; effort: " + live["effort"])
elif command == "send-keys":
    selection = json.loads((root / "selection.json").read_text())
    key = args[-1]
    if key == "Down":
        selection["model"] = "selected-model"
    elif key == "Right":
        selection["effort"] = "high"
    elif key == "s":
        assert args[-3:] == ["-l", "--", "s"], "picker must receive a typed key"
        (root / "live.json").write_text(json.dumps(selection))
    elif key == "Enter":
        for name in ["defaults.json", "live.json"]:
            (root / name).write_text(json.dumps(selection))
    (root / "selection.json").write_text(json.dumps(selection))
