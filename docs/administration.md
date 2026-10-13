# Administration

Runtime status is available from `systemctl status xscs.service` and the HTTP `/readyz` endpoint. Application logs are UTC JSONL files under `ROOT/logs`; `journalctl --utc -u xscs.service` includes startup diagnostics.

The server executable accepts `xscs --root ROOT` and ordinary help/version options. Installation and offline checks, backups and account maintenance use the separate signed XSSI scripts and setup helper. Startup opens existing state and preserves installation secrets.

Normal administrator username/password changes use the person icon in the Web menu. Offline password recovery runs through XSSI maintenance with the service stopped; it validates the current database and preserves `data_dir/secrets.json`, instance identities and encrypted client authorizations. It revokes previous administrator sessions.

Back up the full configured data directory, configuration and explicit external business paths together. Database files and installation secrets belong to the same generation. See [operations](operations.md), [configuration](configuration.md) and [account settings](account-settings.md).
