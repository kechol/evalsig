// Write evalsig canonical records (records/1) as JSON Lines. No dependencies.
//
// Printed by `evalsig schema records --emit typescript`; copy it into a harness so its
// records match the format before they reach evalsig. The writer takes a function that
// receives each line, so it runs anywhere (Node, Deno, Bun, a browser). Check the output
// with `evalsig validate --records <file>`.
//
//   const lines: string[] = [];
//   const w = new RecordsWriter((line) => lines.push(line));
//   w.run({ runId: "triage-v1", systemId: "triage", systemVersion: "prompt-v1", outputType: "binary" });
//   w.judgment({ itemId: "t-1", runId: "triage-v1", genReplicate: 0, outputType: "binary", value: true });

export const RECORDS_VERSION = "records/1";
export const MISSING_REASONS = [
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
] as const;
export type MissingReason = (typeof MISSING_REASONS)[number];
export type OutputType = "binary" | "ordinal" | "score";
export type Source = "human" | "model" | "consensus" | "external";
export type Value = boolean | number;
export interface Missing {
  reason: MissingReason;
  detail?: string;
}

export function missing(reason: MissingReason, detail?: string): Missing {
  if (!MISSING_REASONS.includes(reason)) {
    throw new Error(`unknown missing reason ${reason}`);
  }
  return detail === undefined ? { reason } : { reason, detail };
}

type Json = Record<string, unknown>;

function put(record: Json, fields: Json): Json {
  for (const [k, v] of Object.entries(fields)) {
    if (v !== undefined) record[k] = v;
  }
  return record;
}

function valueOrMissing(record: Json, value?: Value, m?: Missing): Json {
  if ((value === undefined) === (m === undefined)) {
    throw new Error("give exactly one of value and missing");
  }
  if (value !== undefined) {
    if (typeof value === "number" && !Number.isFinite(value)) {
      throw new Error("NaN and infinities are not values; give missing(...) instead");
    }
    record.value = value;
  } else {
    record.missing = m;
  }
  return record;
}

export class RecordsWriter {
  private readonly emit: (line: string) => void;

  constructor(emit: (line: string) => void) {
    this.emit = emit;
  }

  write(record: Json): void {
    this.emit(JSON.stringify(record) + "\n");
  }

  run(r: {
    runId: string;
    systemId: string;
    systemVersion: string;
    outputType: OutputType;
    valueSemantics?: "decision" | "score";
    levels?: number;
    grader?: { id?: string; version?: string; kind?: string };
  }): void {
    const semantics = r.valueSemantics ?? "decision";
    if (semantics === "score" && r.grader === undefined) {
      throw new Error("score runs need a grader");
    }
    this.write(
      put(
        {
          record: "run",
          run_id: r.runId,
          system: { id: r.systemId, version: r.systemVersion },
          value_semantics: semantics,
          output_type: r.outputType,
        },
        { levels: r.levels, grader: r.grader },
      ),
    );
  }

  item(i: {
    itemId: string;
    cluster?: string;
    strata?: Record<string, string | number | boolean>;
    weight?: number;
    occurredAt?: string;
    inputRef?: string;
  }): void {
    this.write(
      put(
        { record: "item", item_id: i.itemId },
        { cluster: i.cluster, strata: i.strata, weight: i.weight, occurred_at: i.occurredAt, input_ref: i.inputRef },
      ),
    );
  }

  label(l: {
    itemId: string;
    raterId: string;
    source: Source;
    value?: Value;
    missing?: Missing;
    version?: number;
    task?: string;
  }): void {
    const record = put(
      { record: "label", item_id: l.itemId, rater_id: l.raterId, source: l.source, version: l.version ?? 1 },
      { task: l.task },
    );
    this.write(valueOrMissing(record, l.value, l.missing));
  }

  judgment(j: {
    itemId: string;
    runId: string;
    genReplicate: number;
    outputType: OutputType;
    value?: Value;
    missing?: Missing;
    levels?: number;
    task?: string;
    gradeReplicate?: number;
  }): void {
    const record = put(
      { record: "judgment", item_id: j.itemId, run_id: j.runId, gen_replicate: j.genReplicate, output_type: j.outputType },
      { levels: j.levels, task: j.task, grade_replicate: j.gradeReplicate },
    );
    this.write(valueOrMissing(record, j.value, j.missing));
  }
}
