# Configuration

`xscs --root ROOT` reads the private JSON file `ROOT/config/config.json`. The root defaults to `/opt/isarmg/xscs` and must be an absolute, normalized directory. Configuration parsing rejects duplicate and unknown fields.

The supplied [example](../config/config.json.example) contains the current product options. `data_dir` defaults to `ROOT/state`; setting it explicitly selects external persistent instance data. `ROOT/logs` and `ROOT/run` remain inside the installation root. State cannot overlap the package, configuration, log, runtime or maintenance paths.

`bind` selects the HTTP listener, `log_level` selects structured logging, and `development` permits loopback HTTP during local development. Production administrator sessions require HTTPS at the browser origin. Only unbound development builds can use directory Web assets.

Product-prefixed environment settings retain their documented precedence over JSON. The public command line selects only `--root`; offline configuration validation reports effective paths and bind address without credentials. The application does not accept proxy or client-IP settings.
