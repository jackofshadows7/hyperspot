# TODO
> Modkit checklist.
---
* [ ] Migrate from per-route `Extension<Arc<_>>` to a process-wide typed state.
  - Avoids layering `Extension(...)` on every route.
  - Keeps handlers strongly typed without ad-hoc clones.
  - Scales across modules without a giant shared struct.
