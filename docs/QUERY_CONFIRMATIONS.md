# Query confirmations

Current source builds add **Settings → Query confirmations**. Preferences save locally with the workspace. Older workspaces use the defaults. Preview 2 predates these controls.

For connections labeled **production**, choose:

| Mode | When the SQL editor asks |
| --- | --- |
| Confirm writes (default) | When SQL analysis classifies the submission as potentially writing |
| Confirm every query | Before each execution, including SELECT |
| Use destructive SQL rules only | When one of the enabled destructive rules matches |

Four independent rules apply on any SQL connection: **DROP**, **TRUNCATE**, **DELETE without WHERE**, and **UPDATE without WHERE**. All are enabled by default. Turning a rule off allows its matching statement to run without that category's prompt; production write/every-query review can still require confirmation. **Restore confirmation defaults** restores production-write review and all four rules.

The confirmation displays the connection, environment and exact submitted SQL. Cancel submits nothing. Run current statement, Run selection, Run All, toolbar actions and keyboard commands share the policy. Preferences are workspace-wide, rather than per connection.

These controls configure review prompts, not database permissions. Native SQL validation and read-only rejection still apply. Unknown warning categories remain visible. ANALYZE always requires its execution confirmation; estimated plans do not execute the target statement. Reviewed table/document edits, Redis writes, imports and reconnect retain their separate confirmations. SQL classification does not prove that a selected function has no side effects.

The source production build, lint and seven focused confirmation/workspace tests pass. Actual Chrome checks exercise the production App with simulated IPC: default production-write review, Cancel without submission, every-query review, one policy-approved submission, saved preferences and reload without replay. This is presentation/dispatch evidence. Updated native desktop/database acceptance and optimized packages remain pending. See [validation](VALIDATION.md#configurable-sql-confirmations--2026-10-03).
