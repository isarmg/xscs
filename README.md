# XSCS

XSCS pairs Sunshine devices, persists authenticated remote operations and provides the administrator control plane.

The server runs on x86_64 GNU/Linux. Its executable has one runtime entry point:

```sh
xscs --root /opt/isarmg/xscs
```

Configuration is `ROOT/config/config.json`. State defaults to `ROOT/state`; an explicit `data_dir` may select external instance data. Logs and transient runtime files always use `ROOT/logs` and `ROOT/run`. Configuration is controlled by the administrator and is read-only to the service account.

Installation, upgrades and offline account maintenance are supplied by the separately maintained [XSSI](https://github.com/isarmg/xssi) scripts and SHA-256-verified `libexec/setup` helper. The server executable only runs the service. Upgrade scripts verify the downloaded package before stopping every writer, then use XSSC for locked backups, current-contract validation, replacement and readiness checks. Uninstallation is managed by the operator.

The server manages application authentication, verified instance and file quotas, and bounded resource use. It does not identify client IP addresses or configure any reverse proxy.

See the [documentation](docs/README.md).
