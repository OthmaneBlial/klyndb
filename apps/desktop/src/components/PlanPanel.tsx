import { useEffect, useState } from "react";
import { Copy, GitBranch } from "lucide-react";
import {
  api,
  type ExecutionPlan,
  type PlanNode,
  type QueryStatus,
} from "../api";

function Node({ node, depth = 0 }: { node: PlanNode; depth?: number }) {
  const [expanded, setExpanded] = useState(depth < 4);
  return (
    <details
      className="plan-node"
      open={expanded}
      onToggle={(e) => setExpanded(e.currentTarget.open)}
    >
      <summary>{node.label}</summary>
      {expanded && node.attributes.length > 0 && (
        <dl className="plan-attributes">
          {node.attributes.map(([name, value]) => (
            <div key={name}>
              <dt>{name}</dt>
              <dd>{value}</dd>
            </div>
          ))}
        </dl>
      )}
      {expanded &&
        node.children.map((child, index) => (
          <Node key={index} node={child} depth={depth + 1} />
        ))}
    </details>
  );
}

export function PlanPanel({ status }: { status: QueryStatus }) {
  const [plan, setPlan] = useState<ExecutionPlan | null>(null);
  const [error, setError] = useState("");
  const [raw, setRaw] = useState(false);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    let live = true;
    if (status.done && !status.error) {
      api("execution_plan", { id: status.id }).then(
        (plan) => {
          if (live) setPlan(plan);
        },
        (error) => {
          if (live) setError(String(error));
        },
      );
    }
    return () => {
      live = false;
    };
  }, [status.id, status.done, status.error]);
  return (
    <div className="plan-panel">
      <div className="plan-heading">
        <div>
          <GitBranch size={18} />
          <h3>
            {status.plan_analyze
              ? "Runtime execution plan"
              : "Estimated execution plan"}
          </h3>
          <span className="plan-kind">
            {status.plan_format?.replaceAll("_", " ")}
          </span>
        </div>
        {plan && (
          <div className="plan-controls">
            <button
              className={!raw ? "selected" : ""}
              aria-pressed={!raw}
              onClick={() => setRaw(false)}
            >
              Tree
            </button>
            <button
              className={raw ? "selected" : ""}
              aria-pressed={raw}
              onClick={() => setRaw(true)}
            >
              Raw
            </button>
            <button
              onClick={() =>
                void navigator.clipboard
                  .writeText(plan.raw)
                  .then(() => setCopied(true))
                  .catch((e) => setError(String(e)))
              }
            >
              <Copy size={13} />
              {copied ? "Copied" : "Copy raw"}
            </button>
          </div>
        )}
      </div>
      <p className="muted plan-note">
        {status.plan_format === "sqlite"
          ? "SQLite reports query structure without runtime timings or costs."
          : "Metrics retain the database's native names and units. Planner cost is separate from elapsed time."}
        {status.plan_analyze &&
          " This statement was executed; ANALYZE does not roll back automatically."}
      </p>
      {(status.error || error) && (
        <p className="error" role="alert">
          {status.error || error}
        </p>
      )}
      {!status.done && (
        <p role="status">
          <span className="spinner" /> Collecting the native plan…
        </p>
      )}
      {plan?.warnings.length ? (
        <details className="plan-warnings" open>
          <summary>Server messages · {plan.warnings.length}</summary>
          {plan.warnings.map((warning, i) => (
            <pre key={i}>{warning}</pre>
          ))}
        </details>
      ) : null}
      {plan &&
        (raw ? (
          <pre className="plan-raw">{plan.raw}</pre>
        ) : (
          <div className="plan-tree">
            {plan.nodes.length ? (
              plan.nodes.map((node, index) => <Node key={index} node={node} />)
            ) : (
              <p>
                No plan operators returned. The raw result remains available.
              </p>
            )}
          </div>
        ))}
    </div>
  );
}
