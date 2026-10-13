# Installation and operation

Use the signed release package with the shared [XSSI](https://github.com/isarmg/xssi) installer. Verify the package against an independently supplied release public key and fingerprint before executing its scripts or helper. The installer prepares the service account, directories, configuration, initial administrator and systemd unit. Host operations belong to the scripts.

The default installation root is `/opt/isarmg/xscs`. Runtime invocation is `xscs --root ROOT`. The server reads `ROOT/config/config.json`, opens existing initialized state and starts HTTP. Startup never creates administrator accounts or installation secrets.

An upgrade first downloads and verifies the complete package while the existing service is running. Its script then stops all writers and delegates the locked backup, explicit source/target schema migration and atomic package switch to XSSC. The script starts the resulting service and checks readiness; failure restores the backed-up generation before returning to the previous service. Media/file paths are preserved instead of silently moved.

Offline configuration checks and administrator password resets use the signed `libexec/setup` helper through XSSI maintenance scripts. Runtime HTTP administrator and health endpoints remain available. Passwords use the existing administrator hash policy; resetting an administrator password preserves instance authorization keys.

Uninstallation and removal of data are performed by the operator.
