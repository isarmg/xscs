# XSCS installation tools

This source belongs to XSCS and is built from the same product commit. It retains
the installation, upgrade, maintenance, integrity, audit and license contracts
imported from XSSI. `UPSTREAM.json` records the original import for attribution.

`support/` contains only the contracts, filesystem safety, schema identity,
secret, envelope and SQLite modules used by the helper. Its import record and
original Apache-2.0 license are retained. No xcss or xcsc repository is fetched.

After verifying and extracting a complete release archive, use its `install.sh`,
`maintenance.sh` and `upgrade.sh`. Each entry provides `--help`; the source-tree
entrypoints here use the same Python implementation. Installation requires an
explicit physical package path, configuration file and installation root:

```sh
sudo ./install.sh --package /absolute/xscs-2.0.0-linux-x86_64 \
  --root /opt/isarmg/xscs --config /absolute/config.json --username admin
```

The initial password is read interactively or from standard input. Installation
does not start or enable the service. The administrator checks the effective
configuration and starts `xscs.service` explicitly. Upgrade and maintenance keep
their writer exclusion, current-contract and recovery requirements.

The frontend receipt verifies retained source inventories and every source file
against the containing product commit. Historical import metadata and original
licenses remain in the delivery; packaging does not download an xcss Git archive.
