# Configuration integrity acceptance

Use a synthetic home, fixture vault and the built CLI. Exercise these failures through the management API and verify the resulting native configuration.

- Saving a display-name edit must preserve the exact original URL, including query and variable references.
- Redacted argv values must stay confidential, survive insertion/reordering and preserve empty arguments. A stale or foreign edit cannot retain another definition's fields.
- A database failure after file replacement must leave an identifiable pending operation. Restart and recovery must finish it exactly once, including a restore interrupted at the same point.
- Replacing an existing configuration preserves its access permissions. A new file and private backup use private permissions. Reject a symlink substituted after preview.
- Earlier operations remain accessible after 26 or more changes. Batch results identify their service and destination.
- A launcher installed under nvm is discoverable with a minimal desktop PATH. A residual configuration alone does not enable writes.
- Exercise the editor, keyboard modal navigation, credential disclosure and source selection at the minimum window size.
