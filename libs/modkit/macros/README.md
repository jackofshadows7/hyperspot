# modkit-macros UI Tests

This crate includes a `trybuild`-based UI test suite under `tests/ui`.

- Run once to (re)generate snapshots:
  
  ```bash
  TRYBUILD=overwrite cargo test -p modkit-macros
  ```

- Regular run (no snapshot changes expected):
  
  ```bash
  cargo test -p modkit-macros
  ```

Notes:
- Keep failing examples minimal so snapshots stay stable.
- Snapshots are committed and validated in CI.
