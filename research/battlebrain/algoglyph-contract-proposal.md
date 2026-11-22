# Proposal to AlgoGlyph: a versioned kernel contract in `flow -- kernels`

RESEARCH-ONLY. This is a handoff, not a change. It was written in the OpenTTD research fork for
the AlgoGlyph maintainers, and nothing in `~/dev/algoglyph` was edited for it. AlgoGlyph is used
from outside only. The observations below come from a read of algoglyph `1b8e009`, the commit of
[`catalogue.json`](../../tools/research/schema/fixtures/battlebrain/catalogue.json).

## Why

Battlebrain resolves each capability it needs to a kernel of the discovered catalogue
(`battlebrain capabilities`, I0098, [README](README.md#capabilities-i0098)). Right now the
catalogue cannot carry that match on its own:

- `src/flow/cli.ts` `runnable()` lists every grid kernel that has a `fromData` with the input
  `"the kernel's data form (see its fromData)"`. That is prose: `knapsack-dp` takes
  `{weights, values, capacity}`, but the catalogue never says so.
- A custom plate's `DataForm.input` (`src/core/solve.ts`) is a shape written for a human reader,
  for example `{from: {x, y, dir: NE|SE|SW|NW}, to: {x, y, dir?}, ...}`. The fork can check that a
  key is declared in it, but it cannot learn a key's type from it.
- No kernel declares its output shape, its units, whether its answer is optimal, or a version.
  A caller learns `result.items`, `result.best`, `result.money` or `result.status` only by reading
  the source.

So Battlebrain keeps an **adapter registry** (`ADAPTERS` in
`tools/research/mcp-rhai/src/battlebrain_contract.rs`). For one kernel id, an adapter states the
capability it satisfies, how each requirement field maps to the kernel's input and output, with
the conversion, whether the answer is exact, and a domain check of the output. The fork owns the
registry, and it is the only explicit kernel list in the fork's contract layer. It retires once
AlgoGlyph publishes what is proposed below.

Since I0228 the fork derives from AlgoGlyph's own source whatever that source already states, and
the adapters keep only the rest. [What the fork derives now](#what-the-fork-derives-now-i0228)
says which fields that covers and which still need AlgoGlyph to publish them.

## Proposal

Add an optional `contract` object to each entry of `flow -- kernels` (and of the `kernels`
command of `flow -- serve`). Existing fields stay as they are, so current callers are unaffected.

```json
{
  "id": "knapsack-dp",
  "title": "Knapsack by dynamic programming",
  "engine": "grid",
  "input": "{weights, values, capacity}",
  "contract": {
    "schema": "algoglyph-kernel-contract/v1",
    "version": "1.0.0",
    "exact": true,
    "input": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "type": "object",
      "required": ["weights", "values", "capacity"],
      "properties": {
        "weights": {"type": "array", "minItems": 1, "maxItems": 200, "items": {"type": "integer", "minimum": 1}, "x-unit": "weight"},
        "values": {"type": "array", "items": {"type": "number", "minimum": 0}, "x-unit": "value"},
        "capacity": {"type": "integer", "minimum": 0, "x-unit": "weight"}
      },
      "x-limits": {"cells": "(capacity + 1) * (items + 1) <= 250000"}
    },
    "output": {
      "type": "object",
      "required": ["best", "items"],
      "properties": {
        "best": {"type": "number", "x-unit": "value"},
        "items": {"type": "array", "items": {"type": "integer", "minimum": 0}, "uniqueItems": true}
      }
    }
  }
}
```

```json
{
  "id": "track-planning",
  "engine": "custom",
  "contract": {
    "schema": "algoglyph-kernel-contract/v1",
    "version": "1.0.0",
    "exact": true,
    "exact_scope": "least cost under its cost model: A* with an admissible heuristic, held to Dijkstra by its tests",
    "input": {
      "type": "object",
      "required": ["from", "to"],
      "properties": {
        "from": {"type": "object", "required": ["x", "y", "dir"], "properties": {"x": {"type": "integer", "x-unit": "tile"}, "y": {"type": "integer", "x-unit": "tile"}, "dir": {"enum": ["NE", "SE", "SW", "NW"]}}},
        "to": {"type": "object", "required": ["x", "y"], "properties": {"x": {"type": "integer", "x-unit": "tile"}, "y": {"type": "integer", "x-unit": "tile"}, "dir": {"enum": ["NE", "SE", "SW", "NW"]}}},
        "map": {"type": "object", "description": "openttd://map data"},
        "prices": {"type": "object", "description": "openttd://economy/prices data; with it, result.money is in game money"}
      }
    },
    "output": {
      "type": "object",
      "required": ["status", "cost", "pieces"],
      "properties": {
        "status": {"enum": ["found", "unreachable", "budget"]},
        "cost": {"type": ["number", "null"], "minimum": 0, "x-unit": "search-cost", "description": "null unless found (src/plates/track-planning/data.ts:252)"},
        "money": {"type": ["number", "null"], "minimum": 0, "x-unit": "money", "description": "only with prices in the input, only when found"},
        "pieces": {"type": "array"}
      }
    }
  }
}
```

### Field rules

- **`schema`** names the contract format, and **`version`** is the kernel's own semver. A change to
  the input or output that is not backwards compatible bumps the major version. A caller pins
  `id@major`, and the invocation envelope (`algoglyph-invocation/v1`) can record it next to the
  catalogue digest and commit it records today.
- **`input` / `output`** are JSON Schema 2020-12, for the data form `flow -- kernel <id> -` reads
  and for the `result` it answers. Each field has its own schema, so a caller can validate before
  the call and after it, without reading the source.
- **`x-unit`** is a unit per field. Abstract kernels use abstract units (`weight`, `value`), and a
  field that shares an `x-unit` with another field must be commensurable with it (`capacity` with
  `weights`). Domain kernels use domain units (`tile`, `money`, `game_days`). The fork maps
  abstract units onto its own `UNITS`, and that mapping, together with conversions such as
  `ceil(money / 1000)`, stays on the caller's side.
- **`exact`** says whether the answer is optimal for the stated problem, and **`exact_scope`**
  qualifies it. Battlebrain ranks exact kernels above heuristic ones, and today an adapter has to
  assert this.
- A graph kernel can declare the same `contract`. Its `shape` and `params` already go most of the
  way.

## What the fork derives now (I0228)

`algoglyph-contracts` (`tools/research/mcp-rhai/src/algoglyph_contracts.rs`) reads the AlgoGlyph
source at the catalogue's commit and writes one `algoglyph-kernel-contract/v1` per kernel to
[`tools/research/schema/derived/algoglyph-contracts.json`](../../tools/research/schema/derived/algoglyph-contracts.json).
Every field names the line or test it comes from, and `absent` lists what no source states
([README](README.md#derived-kernel-contracts-i0228)). The matcher reads engine, input keys and
types, and output keys from it first. Field by field:

| Proposed field | Fork-side now | Read from | Still needs AlgoGlyph |
|---|---|---|---|
| `input` keys and `required` | derived for every kernel | a data form's `input` text (`key?` optional, `a? \| b` alternatives), a grid kernel's data interface, the graph form's doc comment in `src/flow/cli.ts`; `unknownKeys(...)` gives `additionalProperties: false` | a machine-readable form. Nested shapes written as prose stay descriptions. |
| `input` JSON types, enums, defaults | derived where a predicate, an interface, a literal list or a destructuring default states them | `integer`, `finite`, `Array.isArray`, `record`, `typeof`, `every`, `includes`, `length` in the parser and its helpers; refusals read by polarity | a type on every key. A key the parser only forwards has none (for example `track-planning`'s `map` and `prices`). |
| `x-limits` (bounds such as `MAX_ITEMS`, `minimum`) | not derived | the bounds exist only as comparisons and refusal messages | yes |
| `output` keys and types | derived for grid kernels (result interface) and data forms (the record `solve` returns) | literals, `Number.isFinite(v) ? v : null`, fields of a typed call's declared result, typed spreads, input keys echoed, and the keys `test/flow-cli.test.ts` reads | yes for graph kernels, whose result type is their own. Values the source computes untyped also need it (for example `pert-monte-carlo`'s `min` and `max`). |
| `x-unit` | absent | nothing in the source states a unit | yes; until then the adapter asserts units and conversions |
| `exact`, `exact_scope` | absent; each adapter cites the AlgoGlyph test that holds the kernel to a reference | nothing declares optimality | yes |
| `version` | absent; `module.blob` (the git blob of the kernel module) identifies the exact source | no per-kernel version exists | yes, and the matcher's `contract_version` refusal waits for it |

For the two adapted kernels this means:

- **`knapsack-dp`** (derived): input `{weights: integer[], values: number[], capacity: integer}`,
  all required, from `KnapsackData` and the `fromData` checks; output `{best: number, items:
  number[]}`, both required, from `KnapsackResult`. Its catalogue input is prose, so before I0228
  the match recorded its shape as `opaque`; it is now `verified`. **Still hand-kept:** that
  `items` are integer indices (TypeScript has no integer type), the units, the `ceil(money /
  1000)` conversion, exactness, and the domain check.
- **`track-planning`** (derived): `from` and `to` required, each `{x: integer, y: integer, dir:
  NE|SE|SW|NW}` with `to.dir` optional; output `status` (`found`, `unreachable`, `budget`, from
  `Plan.status`), `cost` (`number` or `null`: the source answers null, not Infinity, when nothing
  is found), `money` (`number` or `null`, only with `prices`), `pieces` (an array of `Piece`).
  **Still hand-kept:** the JSON types of the forwarded `map` and `prices` resources, units,
  exactness, and the domain check.

## What the fork would retire

Once a kernel carries `contract/v1`, Battlebrain's matcher reads the field mapping from
`contract.input` and `contract.output`, the units from `x-unit` and the ranking from `exact`. The
keys and types are already read from the derived contract, so a published contract replaces the
derived one with no change to the matcher's checks. The adapter for that kernel then shrinks to
its domain check. The check is still the caller's, because
it tests the answer against the caller's input, for example that the chosen items' value equals
`best`. The adapter is deleted entirely if AlgoGlyph also validates its answers against its own
`output` schema. The matcher refuses a kernel whose contract major version differs from the one
the requirement pins. Its refusal code would be `contract_version`, and the matcher does not
have it yet.

## Not proposed

- A capability vocabulary inside AlgoGlyph. Capabilities such as "portfolio within a budget"
  belong to the caller, and the catalogue only describes kernels.
- Any change to how kernels run, or to their seeded instances.
