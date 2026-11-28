"""Offline synthetic publisher/broker. Only the supplied disposable root is used."""

import argparse
import json
from pathlib import Path


parser = argparse.ArgumentParser()
parser.add_argument("--root", type=Path, required=True)
parser.add_argument("verb", choices=["stage", "publish", "list", "materialize", "credential-state"])
parser.add_argument("--generation", type=int)
parser.add_argument("--provider", choices=["claude"], default="claude")
parser.add_argument("--account", choices=["alpha", "beta"], default="alpha")
parser.add_argument("--format", choices=["json"], default="json")
args = parser.parse_args()
root = args.root.resolve(strict=True)
config = root / args.account
state_file = root / f"publication-{args.account}.json"
time = "2030-01-01T00:00:00Z"
schema = "agent-session.account-broker.v2"


def write(path, value):
    path.write_text(json.dumps(value))
    path.chmod(0o600)


def store_value(generation):
    return {"synthetic": True, "generation": generation}


if args.verb in ("stage", "publish"):
    n = args.generation
    assert n is not None and n > 0
    if args.verb == "stage":
        previous = json.loads(state_file.read_text()) if state_file.exists() else None
        config.mkdir(mode=0o700, exist_ok=True)
        for name in (".credentials.json", "config.json"):
            write(config / name, store_value(n))
        version = {
            "authority_id": "10000000-0000-4000-8000-000000000001",
            "authority_epoch": "20000000-0000-4000-8000-000000000001",
            "generation": str(n),
        }
        snapshot = f"30000000-0000-4000-8000-{n:012}"
        required = {"account_credentials", "account_config", "directory_preparation"}
        stores = [{
            "kind": kind, "required": kind in required,
            "status": "pending" if kind in required else "not_required",
            "verified_at": None,
        } for kind in ["account_credentials", "account_config", "keychain",
                       "directory_preparation", "active_credentials", "proxy_projection"]]
        state = {
            "schema_version": "agent-session.credential-state.v1", "ok": True,
            "provider": "claude", "account": args.account,
            "host_id": "40000000-0000-4000-8000-000000000001",
            "read_at": time, "error": None,
            "state": {
                "authority": {"version": version, "snapshot_id": snapshot,
                              "auth": {"state": "unknown"}},
                "publication": {
                    "status": "publishing", "policy_revision": "1", "target": version,
                    "snapshot_id": snapshot,
                    "attempt_id": f"50000000-0000-4000-8000-{n:012}",
                    "started_at": time, "updated_at": time, "published_at": None,
                    "stores": stores, "failure": None,
                    "last_published": previous["state"]["publication"]["last_published"]
                    if previous else None,
                }, "refresh": None, "authority_contact": {"state": "unknown"},
            },
        }
    else:
        state = json.loads(state_file.read_text())
        publication = state["state"]["publication"]
        assert publication["status"] == "publishing"
        assert publication["target"]["generation"] == str(n)
        assert config.stat().st_mode & 0o777 == 0o700
        for name in (".credentials.json", "config.json"):
            assert json.loads((config / name).read_text()) == store_value(n)
            assert (config / name).stat().st_mode & 0o777 == 0o600
        for store in publication["stores"]:
            if store["required"]:
                store.update(status="verified", verified_at=time)
        publication.update(status="published", published_at=time, last_published={
            "version": publication["target"], "snapshot_id": publication["snapshot_id"],
            "policy_revision": "1", "published_at": time,
        })
    write(state_file, state)
    result = {"synthetic": True, "phase": args.verb, "generation": str(n)}
else:
    with (root / "broker.log").open("a") as log:
        log.write(args.verb + "\n")
    if args.verb == "list":
        result = {"schema_version": schema, "provider": "claude",
                  "capabilities": ["credential_state_v1"], "accounts": [{"account": account} for account in ("alpha", "beta")]}
    elif args.verb == "materialize":
        assert (config / ".credentials.json").is_file()
        result = {"schema_version": schema, "provider": "claude", "account": args.account,
                  "config_dir": str(config)}
    else:
        result = json.loads(state_file.read_text())
print(json.dumps(result))
