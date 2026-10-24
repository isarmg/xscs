# XSCS server foundation

This crate is maintained within the XSCS repository. The initial implementation
was copied from the Linux server support crate at
https://github.com/isarmg/xcss/commit/d9f6f2a900d85eac0bb8c53575ccd4ca09890514.
That URL records history; builds, tests and packages use only these tracked files.

The original Apache-2.0 license is retained in LICENSE. The copied scope is the
Rust library and its build script, SQL schemas, and fixtures included by the
library's existing tests. No standalone upstream services, web applications,
release scripts or CI jobs are imported.

The library keeps the xcss Rust API name for compatibility with existing XSCS
call sites and generated embedded-asset code. Its Cargo package is the private
xscs-foundation package, resolved through this repository's workspace path.
Release provenance is the XSCS source commit, rather than an upstream checkout.

The product schema composer and server-control-plane profile are also retained
for offline database-schema maintenance. Its generated SQL header remains
unchanged to preserve the existing schema identity.

Run the library tests with `-- --test-threads=2`, matching the original CI
configuration at the imported revision. Independent authentication fixtures
share the same process-wide, two-slot Argon2 admission budget. This test-harness
setting does not change production concurrency, timeouts or assertions.
