# steputils and its two dependencies (vendored wheels)

These are the wheels of the third-party Part 21 parser the STEP export tests (`tests/step_19_reader.rs`)
run in a container, byte for byte what PyPI served on 2026-10-24. `SHA256SUMS` lists their digests and the
test refuses to run if one has changed.

| wheel | version | licence | what it is |
|---|---|---|---|
| `steputils` | 0.1 | MIT | ISO 10303-21 reader and writer, the independent implementation the tests record |
| `pyparsing` | 3.3.3 | MIT | the parser library `steputils` imports |
| `antlr4_python3_runtime` | 4.13.2 | BSD | the ANTLR runtime `steputils` imports |

All three are pure Python wheels. I vendor them because the validation container has no network (and a
test that installs its reader at run time reads whatever the index says that day). Python imports a wheel
straight from the zip, so the test sets `PYTHONPATH` to the three files and installs nothing.
