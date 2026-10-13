# XSCS

XSCS pairs Sunshine devices, persists authenticated remote operations and provides the administrator control plane.

The server runs on x86_64 GNU/Linux. Its executable has one runtime entry point:

```sh
xscs --root /opt/isarmg/xscs
```

Configuration is `ROOT/config/config.json`. State defaults to `ROOT/state`; an explicit `data_dir` may select external instance data. Logs and transient runtime files always use `ROOT/logs` and `ROOT/run`. Configuration files are owned by the service account, with directory mode 0700 and file mode 0600.

Installation, upgrades and offline account maintenance are supplied by the separately maintained [XSSI](https://github.com/isarmg/xssi) scripts and signed `libexec/setup` helper. The server executable only runs the service. Upgrade scripts verify the downloaded package before stopping every writer, then use XSSC for locked backups, declared migration, replacement and readiness checks. Uninstallation is managed by the operator.

The server manages application authentication, verified instance and file quotas, and bounded resource use. It does not identify client IP addresses or configure any reverse proxy.

See [configuration](docs/configuration.md), [operations](docs/operations.md), [development](docs/development.md) and [feature inventory](docs/feature-inventory-and-tradeoffs.md).
