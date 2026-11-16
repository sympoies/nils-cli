"""Inventory-only mirror of lib.rs title normalization/rendering predicates.

This never accepts or rewrites runtime state. Keep release-derived examples in
sync with structured_title_v122_compatibility_outputs_match_release_fixtures.
"""

import re

JS_SPACE = "\u0009\u000a\u000b\u000c\u000d\u0020\u00a0\u1680" + "".join(
    chr(c) for c in range(0x2000, 0x200b)
) + "\u2028\u2029\u202f\u205f\u3000\ufeff"
REFERENCE = re.compile(r"(?:[a-z][a-z0-9_.-]{1,63} )?#[1-9][0-9]{0,9}")


def component(value):
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError()
    value = " ".join(part for part in re.split("[" + JS_SPACE + "]+", value) if part)
    if len(value) > 120:
        raise ValueError()
    return value or None


def normalize(state):
    if not isinstance(state, dict):
        raise ValueError()
    topic, activity = component(state.get("topic")), component(state.get("activity"))
    source = state.get("topic_source")
    if not ((source == "none" and topic is None)
            or (source in ("auto", "user") and topic is not None)):
        raise ValueError()
    refs = state.get("references", [])
    if not isinstance(refs, list) or len(refs) > 2:
        raise ValueError()
    normalized = []
    for ref in refs:
        if not isinstance(ref, str) or not REFERENCE.fullmatch(ref.strip(JS_SPACE)):
            raise ValueError()
        ref = ref.strip(JS_SPACE)
        if ref not in normalized:
            normalized.append(ref)
    return topic, activity, normalized


def truncate(value, limit):
    if len(value) <= limit:
        return value
    return value[:limit] if limit <= 3 else value[:limit - 3] + "..."


def contains(title, reference, old):
    # Rust match_indices yields non-overlapping matches.
    for match in re.finditer(re.escape(reference), title):
        tail = title[match.end():match.end() + 1]
        if not tail or not (tail in "0123456789" if old else
                            tail.isascii() and (tail.isalnum() or tail == "_")):
            return True
    return False


def render(state, old=False):
    topic, activity, refs = normalize(state)
    limit = 85 if activity is not None else 120
    visible = truncate(topic, limit) if topic else None
    references = ""
    if topic:
        for _ in range(len(refs) + 1):
            references = " ".join(ref for ref in refs if not contains(visible, ref, old))
            budget = max(0, limit - len(references) - 1) if references else limit
            next_visible = truncate(topic, budget)
            if visible == next_visible:
                break
            visible = next_visible
    else:
        references = " ".join(refs)
    anchor = (visible + " " + references if references else visible) if visible else (
        truncate(references, limit) if references else None
    )
    if anchor is None:
        return truncate(activity, 120) if activity else None
    if activity is None:
        return truncate(anchor, 120)
    return anchor + " - " + truncate(activity, max(0, 120 - len(anchor) - 3))


def classify(record):
    if record.get("title_state") is None:
        return "unstructured"
    try:
        title = record.get("title")
        if title is not None and not isinstance(title, str):
            raise ValueError()
        # Stored-pair equality follows effective_session_title_state, without
        # trimming the stored title as the request canonicalizer does.
        if title == render(record["title_state"]):
            return "current"
        if title == render(record["title_state"], old=True):
            return "old_only"
        return "mismatch"
    except ValueError:
        return "invalid"
