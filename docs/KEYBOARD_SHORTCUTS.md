# Keyboard shortcuts

Current source builds let you change twelve workspace shortcuts in **Settings → Keyboard shortcuts**. Downloadable [Preview 2](RELEASES.md) predates this preference.

Focus a binding and press the combination you want. Use Cmd/Ctrl with a letter, number or Enter, or F1–F12; Shift and Alt are optional modifiers. The recorded key follows your keyboard layout. Some combinations are handled by the operating system before the app receives them; choose another combination if recording does not respond. Cmd and Ctrl both invoke the app's `Mod` bindings; hints show Cmd on macOS and Ctrl elsewhere.

| Action | Default |
| --- | --- |
| Command palette | Cmd/Ctrl + K |
| New tab | Cmd/Ctrl + T |
| Run current statement or selection | Cmd/Ctrl + Enter |
| Format SQL | Cmd/Ctrl + Shift + F |
| Save query | Cmd/Ctrl + S |
| New connection, Run entire editor, Refresh schema, Saved queries, Query history, Toggle sidebar, Settings | Unassigned |

**Clear** disables a binding; its button and command-palette entry remain available. A cleared or remapped combination may return to its usual editor or operating-system behavior. **Restore shortcut defaults** restores the table above. Duplicate bindings are rejected with the action name. Standard editing and window combinations, including Select all, Copy/Paste, Undo/Redo, Find and Quit, are reserved. Plain typing, Tab and Escape remain available to the editor and dialogs.

Changes save in the local workspace and survive a full restart. Shortcut hints in the command palette, Run button and SQL controls follow your preferences. Older workspaces receive the defaults; malformed or conflicting stored maps fall back to defaults.

Run uses the selected SQL when a selection exists, otherwise the current statement. Run entire editor submits the full draft through the same execution pipeline. Assigning shortcuts does not change connection requirements, row limits, timeouts, query review, read-only enforcement or staged-edit guards. Holding a key does not repeatedly invoke an action. Shortcuts are suspended while a dialog is open, so recording a binding cannot execute SQL.

The Run, Run entire editor, Format, Save query and Refresh schema bindings apply to SQL tabs. MongoDB and Redis retain their dedicated workspace execution controls and existing shortcuts; the shared workspace actions still apply. Nothing runs automatically when preferences restore.

Actual Chrome checks cover the production app with explicit simulated IPC, collision rejection, recording, disabling/restoring defaults, changing hints, reload persistence, current/selection/all dispatch, dialog guards, disconnected execution and confirmation cancellation. The actual macOS arm64 source debug app verifies recording, native SQLite current/selection/all execution once each, obsolete-binding suppression, full Quit/relaunch persistence and no query replay. Native Windows/Linux and other engines' remapped execution remain pending. See [validation evidence](VALIDATION.md#customizable-keyboard-shortcuts--2026-10-03).
