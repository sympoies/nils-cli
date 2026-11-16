"""Fixed aggregate predicates for the operator-run state retirement probe.

These are shape counts, not runtime admission or a migration implementation.
Unknown strings always enter fixed buckets; never use persisted values as keys.
"""

import re

from retirement_titles import classify


def uint(value, bits=64):
    return type(value) is int and 0 <= value < 2**bits


def buckets(*names):
    return dict.fromkeys(names, 0)


def presence(obj, key, valid):
    if key not in obj:
        return "missing"
    if obj[key] is None:
        return "null"
    return "present" if valid(obj[key]) else "invalid"


def schema(value, prefix, versions):
    result = buckets("missing", "null", "invalid", "other", *versions)
    key = presence(value, "schema_version", lambda v: isinstance(v, str))
    if key == "present":
        key = next((v for v in versions if value["schema_version"] == prefix + v), "other")
    result[key] = 1
    return result


def session_counts():
    return {
        "records": 0,
        "title_revision": buckets("missing", "null", "present", "invalid"),
        "session_incarnation": buckets("missing", "null", "present", "invalid"),
        "title_pairs": buckets("unstructured", "current", "old_only", "mismatch", "invalid"),
        "switch_intents": {
            "absent": 0, "invalid": 0,
            **{s: buckets("missing", "null", "present", "invalid")
               for s in ("queued", "applying", "failed")},
        },
        "retitle_markers": {
            "absent": 0, "invalid": 0, "receipts": 0,
            "schema": buckets("missing", "null", "invalid", "other", "v2"),
            "observation": buckets("missing", "null", "present", "invalid"),
        },
    }


def add_session(result, record):
    if not isinstance(record, dict) or record.get("schema_version") != "agent-session.session.v1":
        raise ValueError()
    if not isinstance(record.get("agent"), str) or not record["agent"]:
        raise ValueError()
    result["records"] += 1
    result["title_revision"][presence(record, "title_revision", uint)] += 1
    runtime = record.get("runtime")
    if runtime is None:
        incarnation = "missing"
    elif not isinstance(runtime, dict):
        incarnation = "invalid"
    else:
        incarnation = presence(runtime, "launch_id", lambda v: isinstance(v, str) and bool(v))
    result["session_incarnation"][incarnation] += 1
    result["title_pairs"][classify(record)] += 1
    intents = result["switch_intents"]
    if "codex_account_next" not in record:
        intents["absent"] += 1
    else:
        intent = record["codex_account_next"]
        # Count the requested state/ID shape independently of account values.
        # This does not certify that the runtime would admit the full intent.
        if (not isinstance(intent, dict) or not isinstance(intent.get("state"), str)
                or intent["state"] not in ("queued", "applying", "failed")
                or intent.get("schema_version") != "agent-session.codex-account-next.v1"):
            intents["invalid"] += 1
        else:
            intents[intent["state"]][presence(
                intent, "intent_id", lambda v: isinstance(v, str)
                and re.fullmatch(r"[A-Za-z0-9-]{1,128}", v) is not None
            )] += 1
    markers = result["retitle_markers"]
    if "session_retitle_v2" not in record:
        markers["absent"] += 1
        return
    marker = record["session_retitle_v2"]
    if not isinstance(marker, dict):
        markers["invalid"] += 1
        return
    for key, n in schema(marker, "agent-session.session-retitle-state.", ("v2",)).items():
        markers["schema"][key] += n
    receipts = marker.get("receipts", [])
    if not isinstance(receipts, list) or any(not isinstance(r, dict) for r in receipts):
        markers["invalid"] += 1
        return
    markers["receipts"] += len(receipts)
    for receipt in receipts:
        markers["observation"][presence(receipt, "observation", lambda v: isinstance(v, dict))] += 1


def auto_counts():
    return {"absent_files": 0, "invalid_records": 0, "other_records": 0,
            "recovery_terminals": buckets("claude", "codex", "other")}


def add_auto(result, state, provider):
    if (not isinstance(state, dict)
            or state.get("schema_version") != "agent-session.auto-resume.v1"
            or type(state.get("enabled")) is not bool
            or not isinstance(state.get("state"), str)
            or not isinstance(state.get("updated_at"), str)
            or state.get("failure_reason") is not None and not isinstance(state["failure_reason"], str)
            or state.get("blocked_turn_id") is not None and not isinstance(state["blocked_turn_id"], str)
            or state.get("blocked_revision") is not None and not uint(state["blocked_revision"])):
        result["invalid_records"] += 1
    elif (state["enabled"] is False and state["state"] == "terminal_failure"
          and state.get("failure_reason") == "usage_window_not_exhausted"
          and state.get("blocked_turn_id") is not None
          and state.get("blocked_revision") is not None):
        result["recovery_terminals"][provider if provider in ("claude", "codex") else "other"] += 1
    else:
        result["other_records"] += 1


def collection(obj, key, kind):
    value = obj.get(key, kind())
    if not isinstance(value, kind):
        raise ValueError()
    rows = list(value.values()) if kind is dict else value
    if any(not isinstance(row, dict) for row in rows):
        raise ValueError()
    return rows


def registry_counts(registry):
    result = {"schema": schema(registry, "agent-session.coordination-registry.", ("v1", "v2", "v3"))}
    operations = collection(registry, "operations", list)
    ops = {"total": len(operations), "other": 0, "invalid": 0,
           "expired_terminal_at": buckets("missing", "null", "present", "invalid")}
    for op in operations:
        state = op.get("state")
        if state == "expired":
            ops["expired_terminal_at"][presence(op, "terminal_at_epoch", lambda v: type(v) is int)] += 1
        elif state in ("active", "completing", "reconcile_pending", "completed", "failed", "abandoned"):
            ops["other"] += 1
        else:
            ops["invalid"] += 1
    result["operations"] = ops
    brokers = collection(registry, "brokers", dict)
    result["brokers"] = {
        "total": len(brokers),
        "runtime_identity": buckets("missing", "null", "present", "invalid"),
        "binary_version": buckets("missing", "null", "present", "invalid"),
    }
    for broker in brokers:
        for key, valid in (("runtime_identity", lambda v: isinstance(v, dict)),
                           ("binary_version", lambda v: isinstance(v, str))):
            result["brokers"][key][presence(broker, key, valid)] += 1
    result["completion_events"] = len(collection(registry, "completion_events", list))
    messages = collection(registry, "messages", list)
    result["mailbox"] = {"total": len(messages),
                         "millis": buckets("missing", "zero", "positive", "invalid"),
                         "seconds": buckets("missing", "null", "present", "invalid")}
    for message in messages:
        value = message.get("created_at_epoch_millis")
        key = "missing" if "created_at_epoch_millis" not in message else (
            "zero" if type(value) is int and value == 0 else
            "positive" if type(value) is int and 0 < value < 2**63 else "invalid")
        result["mailbox"]["millis"][key] += 1
        result["mailbox"]["seconds"][presence(
            message, "created_at_epoch", lambda v: type(v) is int and -2**63 <= v < 2**63
        )] += 1
    return result


def heartbeat_counts(state):
    pending = state.get("completion_pending")
    return {
        "schema": schema(state, "agent-session.broker-state.", ("v1",)),
        "owners": len(collection(state, "owners", dict)),
        "completion_pending": "true" if pending is True else "false" if pending is False else "invalid",
        "completion_registry": presence(state, "completion_registry", lambda v: isinstance(v, dict)),
    }


def journal_counts(journal):
    outbox = collection(journal, "remote_outbox", list)
    retained = collection(journal, "retained", list)
    result = {
        "schema": schema(journal, "agent-session.federation-journal.", tuple(f"v{i}" for i in range(1, 7))),
        "outbox": len(outbox), "retained": len(retained), "queued": 0,
        "invalid_envelopes": 0,
        "envelope_schema": buckets("missing", "null", "invalid", "other", "session_v1", "session_v2", "service_v2"),
        "category": buckets("missing", "null", "invalid", "other", "uncategorized", "progress", "handoff", "blocker", "decision", "report"),
        "forwarding": buckets("missing", "null", "present", "invalid"),
    }
    names = {"agent-session.remote-message.v1": "session_v1",
             "agent-session.remote-message.v2": "session_v2",
             "agent-session.remote-service-message.v2": "service_v2"}
    for item in outbox:
        result["queued"] += item.get("state") == "queued"
        envelope = item.get("envelope")
        if not isinstance(envelope, dict):
            result["invalid_envelopes"] += 1
            continue
        key = presence(envelope, "schema_version", lambda v: isinstance(v, str))
        if key == "present":
            key = names.get(envelope["schema_version"], "other")
        result["envelope_schema"][key] += 1
        key = presence(envelope, "category", lambda v: isinstance(v, str))
        if key == "present":
            key = envelope["category"] if envelope["category"] in result["category"] and envelope["category"] not in ("missing", "null", "invalid", "other") else "other"
        result["category"][key] += 1
        result["forwarding"][presence(envelope, "forwarding", lambda v: isinstance(v, dict))] += 1
    return result
