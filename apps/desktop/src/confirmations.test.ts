import { describe, expect, it } from "vitest";
import { defaultConfirmations, queryConfirmation } from "./confirmations";
import { defaults, restoreWorkspace, serializeWorkspace } from "./workspace";
const write = {
  statements: ["UPDATE t SET x=1"],
  read_only: false,
  warnings: ["UPDATE has no WHERE clause"],
};
const read = { statements: ["SELECT 1"], read_only: true, warnings: [] };

describe("query confirmations", () => {
  it("restores safe defaults for old workspaces and persists explicit choices", () => {
    expect(
      restoreWorkspace({ preferences: {} }).preferences.confirmations,
    ).toEqual(defaultConfirmations);
    expect(
      restoreWorkspace({
        preferences: {
          confirmations: { update: "false", production: "invalid" },
        },
      }).preferences.confirmations,
    ).toEqual(defaultConfirmations);
    const preferences = {
      ...defaults,
      confirmations: {
        ...defaultConfirmations,
        update: false,
        production: "all" as const,
      },
    };
    expect(
      restoreWorkspace(serializeWorkspace([], "", preferences)).preferences
        .confirmations,
    ).toEqual(preferences.confirmations);
  });
  it("can acknowledge a disabled category, while production and mandatory warnings still prompt", () => {
    const policy = { ...defaultConfirmations, update: false };
    expect(queryConfirmation(write, "development", policy)).toEqual({
      warnings: [],
      confirmedByPolicy: true,
    });
    expect(queryConfirmation(write, "production", policy).warnings).toEqual([
      "This statement may write to production.",
    ]);
    expect(
      queryConfirmation(write, "production", {
        ...policy,
        production: "destructive",
      }).confirmedByPolicy,
    ).toBe(true);
    for (const plan of [undefined, "analyze"] as const) {
      const result = queryConfirmation(
        {
          ...write,
          warnings: [...write.warnings, "ANALYZE executes side effects"],
        },
        "development",
        policy,
        plan,
      );
      expect(result.warnings).toContain("ANALYZE executes side effects");
      expect(result.confirmedByPolicy).toBe(false);
    }
  });
  it("supports every-production-query review and keeps estimated plans nonexecuting", () => {
    expect(
      queryConfirmation(read, "production", defaultConfirmations).warnings,
    ).toEqual([]);
    const policy = { ...defaultConfirmations, production: "all" as const };
    expect(queryConfirmation(read, "production", policy).warnings).toEqual([
      "You are querying a production database.",
    ]);
    expect(queryConfirmation(write, "production", policy, "estimate")).toEqual({
      warnings: [],
      confirmedByPolicy: false,
    });
  });
});
