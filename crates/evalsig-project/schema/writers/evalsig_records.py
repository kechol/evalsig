"""Write evalsig canonical records (records/1) as JSON Lines. Standard library only.

Printed by `evalsig schema records --emit python`; copy it into a harness so its records
match the format before they reach evalsig. Check the output with
`evalsig validate --records <file>`.

    from evalsig_records import Writer, missing
    with open("records.jsonl", "w", encoding="utf-8") as f:
        w = Writer(f)
        w.run("triage-v1", system_id="triage", system_version="prompt-v1", output_type="binary")
        w.item("t-1", cluster="thread-1")
        w.label("t-1", rater_id="rater:a", source="human", value=True)
        w.judgment("t-1", "triage-v1", gen_replicate=0, output_type="binary", value=False)
        w.judgment("t-1", "triage-v1", gen_replicate=1, output_type="binary", missing=missing("timeout"))
"""
import json
import math

RECORDS_VERSION = "records/1"
MISSING_REASONS = (
    "error",
    "timeout",
    "refused",
    "invalid_output",
    "limit",
    "grader_failed",
    "not_run",
    "invalidated",
    "excluded",
    "other",
)
OUTPUT_TYPES = ("binary", "ordinal", "score")
SOURCES = ("human", "model", "consensus", "external")


def missing(reason, detail=None):
    """A missing value with one of MISSING_REASONS."""
    if reason not in MISSING_REASONS:
        raise ValueError(f"unknown missing reason {reason!r}; use one of {MISSING_REASONS}")
    m = {"reason": reason}
    if detail is not None:
        m["detail"] = str(detail)
    return m


def _put(record, **fields):
    for k, v in fields.items():
        if v is not None:
            record[k] = v
    return record


def _value_or_missing(record, value, missing_value):
    if (value is None) == (missing_value is None):
        raise ValueError("give exactly one of value and missing")
    if value is not None:
        if isinstance(value, float) and not math.isfinite(value):
            raise ValueError("NaN and infinities are not values; give missing(...) instead")
        record["value"] = value
    else:
        record["missing"] = missing_value
    return record


class Writer:
    """Writes one record per line to a text file opened for writing."""

    def __init__(self, f):
        self.f = f

    def write(self, record):
        self.f.write(json.dumps(record, ensure_ascii=False, sort_keys=True, allow_nan=False) + "\n")

    def run(self, run_id, system_id, system_version, output_type, value_semantics="decision",
            levels=None, grader=None, features=None):
        if output_type not in OUTPUT_TYPES:
            raise ValueError(f"output_type is one of {OUTPUT_TYPES}")
        if value_semantics == "score" and grader is None:
            raise ValueError("score runs need a grader, e.g. {'id': 'rubric', 'version': '1'}")
        self.write(_put({"record": "run", "run_id": run_id,
                         "system": {"id": system_id, "version": system_version},
                         "value_semantics": value_semantics, "output_type": output_type},
                        levels=levels, grader=grader, features=features))

    def item(self, item_id, cluster=None, strata=None, weight=None, occurred_at=None,
             input_ref=None, reference_ref=None, attrs=None):
        self.write(_put({"record": "item", "item_id": item_id}, cluster=cluster, strata=strata,
                        weight=weight, occurred_at=occurred_at, input_ref=input_ref,
                        reference_ref=reference_ref, attrs=attrs))

    def label(self, item_id, rater_id, source, value=None, missing=None, version=1, task=None,
              reason=None, labeled_at=None, levels=None):
        if source not in SOURCES:
            raise ValueError(f"source is one of {SOURCES}")
        record = _put({"record": "label", "item_id": item_id, "rater_id": rater_id,
                       "source": source, "version": version},
                      task=task, reason=reason, labeled_at=labeled_at, levels=levels)
        self.write(_value_or_missing(record, value, missing))

    def judgment(self, item_id, run_id, gen_replicate, output_type, value=None, missing=None,
                 levels=None, task=None, grade_replicate=None, features=None, output_ref=None,
                 judged_at=None):
        if output_type not in OUTPUT_TYPES:
            raise ValueError(f"output_type is one of {OUTPUT_TYPES}")
        record = _put({"record": "judgment", "item_id": item_id, "run_id": run_id,
                       "gen_replicate": gen_replicate, "output_type": output_type},
                      levels=levels, task=task, grade_replicate=grade_replicate,
                      features=features, output_ref=output_ref, judged_at=judged_at)
        self.write(_value_or_missing(record, value, missing))

    def attribution(self, item_id, run_id, rater_id, causes, task=None):
        """The causes a rater gave to the run's error on the item (D-048)."""
        if isinstance(causes, str) or not causes:
            raise ValueError("causes is a non-empty list of strings")
        self.write(_put({"record": "attribution", "item_id": item_id, "run_id": run_id,
                         "rater_id": rater_id, "causes": list(causes)}, task=task))
