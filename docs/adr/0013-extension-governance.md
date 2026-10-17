# ADR 0013: Extension governance mechanics

Status: Accepted, 2026-10-17. Decides P3.5 (`docs/EXTENSIONS.md`, `docs/extensions/`, the gate in
`pvec-capabilities`). Builds on ADR 0008 and ADR 0010.

## Context

ADR 0008 fixed the container rule: a geometry kind is never added by a vendor block, it needs an ADR and a
section of the profile. Governance needs a gate that is more than a sentence in a document, or the only
claim PVec makes about its mathematics rests on reviewer discipline.

## Options

| question | options | evidence |
|---|---|---|
| where the proposal lives | A. a typed struct filled in by code. B. a markdown file with fixed headings that a checker reads | B is what a human writes and reviews, and a committed sample and template can be tested the same way as the code (the sample passes end to end, deleting any one section of it makes it fail with that section named). A would make the proposal a code change with no document to argue about |
| how strict the checker is | A. headings present. B. headings present, bodies not empty, no `TODO`, at least a few words, and structured entries where an argument needs one (a Part 42 entry has a construct and a reason, a fixture has an evidence class) | with A an empty section passes, which is the failure the issue exists to prevent. The thresholds are small on purpose (eight words for prose, three for a reason): they separate a label from an argument without trying to judge the argument |
| STEP exactness | A. take the proposal's word. B. an exact claim needs a fixture that records `round-trip: passing` | the issue says an exact STEP behaviour needs a passing round-trip fixture; B makes that checkable. Whether the round trip really passed is phase 6's evidence, so the checker only demands that the proposal records it |
| required extensions | A. a flag. B. required only with an ADR listed in the entry, and a file cannot declare an optional extension required | a required capability makes every reader without it refuse the file, which is the cost ADR 0008 accepts only for a decided reason |

## Decision

B in every row. `Class::Extension` and an `adr` field join the registry; `Registry::register_extension`
is the only way in and registers nothing unless the proposal is complete, the id is the next dense one,
the name is new, no ISO construct is named, a required extension lists an ADR, and the STEP behaviour in
the entry agrees with the proposal. EPIC O's four evidence class names are the vocabulary and a fifth is
refused. The checker reports every problem it finds, so a missing artifact is never hidden behind another.

## Consequences

* The generated `docs/CAPABILITIES.md` gains an ADR column.
* The sample in `docs/extensions/sample-proposal.md` is marked as a sample, is not registered, and exists
  as the passing example of the gate.
* The checker is text based. It does not judge whether an argument is correct, only that one is there and
  has the required shape; review still decides whether it is a good one.

## Reversal threshold

Move to a typed proposal if the first real extension shows the markdown shape is awkward to produce, or if
two proposals pass the checker with arguments a reviewer finds empty (then the thresholds, not the format,
are what to change).
