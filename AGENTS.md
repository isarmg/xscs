# XSCS development rules

The Rust workspace contains the Linux server, the shared device protocol and
the repository-owned foundation. Read `docs/development.md` before validation.

* Parse inputs at HTTP/WebSocket, configuration and persistent-storage boundaries.
  Retain validated types in internal calls and avoid parsing them again.
* Read only the fields needed for the operation. Building an access policy must
  not require parsing unrelated diagnostic snapshots or capabilities.
* Session ownership, revocation and authorization leases are mutable facts.
  Preserve their current-state checks and transaction boundaries.
* Propagate database and I/O errors with their sources. Return `NotFound` only
  for a missing record, and distinguish corrupt persisted data from absence.
* Prefer borrowing or moving to cloning whole records or collections. Keep
  serialization at boundaries and blocking work outside async runtime threads.
* External failures use `Result`; express internal invariants through types or
  direct control flow. A proven invariant may use `expect("invariant: ...")`.
* Keep changes local. Preserve wire formats, schema identity, durable result
  acknowledgements, secret redaction and source/license records.
* Keep authentication, CSRF/origin, input bounds, path/FFI safety and resource
  limits. Decide whether a check is redundant from its full call chain.
* Do not apply blanket replacements of `checked_*`, `saturating_*`, `.ok()` or
  `validate_*`. Review their source, failure semantics and callers individually.
* Add behavior tests for changed boundaries and errors. Use existing fixtures;
  do not introduce production hooks or a second implementation just for tests.

Run `cargo fmt --all -- --check`, the affected tests, and
`cargo clippy --locked --workspace --all-targets -- -D warnings`.
Server tests use `--test-threads=2` for the shared password-hashing fixtures.
Record any environment limitations and the existing runtime paths changed.

See [the simplification rules and implementation record](docs/defensive-programming.md).
