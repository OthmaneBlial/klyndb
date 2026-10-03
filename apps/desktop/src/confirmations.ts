import type { Analysis } from "./api";

export const destructiveConfirmations = [
  {
    id: "drop",
    label: "DROP objects",
    warning: "DROP removes database objects",
  },
  {
    id: "truncate",
    label: "TRUNCATE tables",
    warning: "TRUNCATE removes all rows",
  },
  {
    id: "delete",
    label: "DELETE without WHERE",
    warning: "DELETE has no WHERE clause",
  },
  {
    id: "update",
    label: "UPDATE without WHERE",
    warning: "UPDATE has no WHERE clause",
  },
] as const;
export interface Confirmations {
  production: "writes" | "all" | "destructive";
  drop: boolean;
  truncate: boolean;
  delete: boolean;
  update: boolean;
}
export const defaultConfirmations: Confirmations = {
  production: "writes",
  drop: true,
  truncate: true,
  delete: true,
  update: true,
};
export function restoreConfirmations(value: unknown): Confirmations {
  const saved =
    value && typeof value === "object" ? (value as Partial<Confirmations>) : {};
  return {
    production:
      saved.production === "all" || saved.production === "destructive"
        ? saved.production
        : "writes",
    drop: saved.drop !== false,
    truncate: saved.truncate !== false,
    delete: saved.delete !== false,
    update: saved.update !== false,
  };
}
export function queryConfirmation(
  analysis: Analysis,
  environment: string,
  preferences: Confirmations,
  plan?: "estimate" | "analyze",
) {
  if (plan === "estimate") return { warnings: [], confirmedByPolicy: false };
  const warnings = analysis.warnings.filter((warning) => {
    const kind = destructiveConfirmations.find(
      (item) => item.warning === warning,
    );
    // Unrecognized warnings always remain visible, including ANALYZE side effects.
    return !kind || preferences[kind.id];
  });
  if (
    environment === "production" &&
    (preferences.production === "all" ||
      (preferences.production === "writes" && !analysis.read_only))
  )
    warnings.unshift(
      analysis.read_only
        ? "You are querying a production database."
        : "This statement may write to production.",
    );
  if (plan === "analyze")
    warnings.unshift(
      "ANALYZE executes this statement, including writes and side effects. It does not roll back automatically.",
    );
  return {
    warnings,
    // The native guard still validates SQL/read-only access. A disabled category
    // acknowledges only its known warning, never a new or mandatory warning.
    confirmedByPolicy:
      !plan && !warnings.length && analysis.warnings.length > 0,
  };
}
