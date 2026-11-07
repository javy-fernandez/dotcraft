# Workspace edit safety

Workspace edits containing file creation, renaming, or deletion are rejected before any text edits are applied. The language server receives `applied: false` and a failure reason, and the user sees an error. Code action commands do not execute after a rejected edit. Client capabilities no longer advertise unsupported resource operations.

Validation: `cargo test -p rc-app workspace_ --offline -- --nocapture` passed two tests. Mixed edits containing each resource operation leave both an open buffer and a closed file unchanged; ordinary text-only operations still apply successfully.

This prevents partially applying refactorings that contain unsupported resource operations. Transactional resource edits, global undo, version validation, and broader rollback support remain pending.
