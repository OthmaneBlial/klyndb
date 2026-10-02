import { invoke } from "@tauri-apps/api/core";
export interface Connection {
  id: string;
  name: string;
  engine: string;
  address: string;
  environment: string;
  group: string;
  color: string;
  favorite: boolean;
  read_only: boolean;
  create_file: boolean;
}
export interface Capabilities {
  transactions: boolean;
  schemas: boolean;
  explain: boolean;
  explain_analyze: boolean;
  edit_rows: boolean;
  cancel: boolean;
  tls: boolean;
}
export interface Table {
  schema: string;
  name: string;
  kind: string;
}
export interface Column {
  name: string;
  data_type: string;
  nullable: boolean;
  primary_key: boolean;
  default: string | null;
  generated: boolean;
}
export interface TableInfo {
  editable: boolean;
  columns: Column[];
  ddl: string | null;
  indexes: Record<string, unknown>[];
  foreign_keys: Record<string, unknown>[];
}
export type Cell =
  | { kind: "null" }
  | { kind: "text" | "number" | "binary"; value: string }
  | { kind: "boolean"; value: boolean }
  | { kind: "json"; value: unknown };
export type Row = Cell[];
export type Change =
  | { kind: "insert"; values: Record<string, Cell> }
  | { kind: "update"; old: Row; values: Record<string, Cell> }
  | { kind: "delete"; old: Row };
export interface MutationResult {
  affected: number;
  pending_transaction: boolean;
}
export interface ResultSet {
  columns: string[];
  rows: number;
  affected: number;
  truncated: boolean;
}
export interface QueryStatus {
  id: string;
  sets: ResultSet[];
  done: boolean;
  error: string | null;
  elapsed_ms: number;
  connection_id: string;
  transaction: "idle" | "active" | "failed" | null;
  plan_format: PlanFormat | null;
  plan_analyze: boolean;
}
export type PlanFormat =
  "sqlite" | "postgres_json" | "mysql_json" | "mysql_tree" | "maria_json";
export interface PlanNode {
  label: string;
  attributes: [string, string][];
  children: PlanNode[];
}
export interface ExecutionPlan {
  format: PlanFormat;
  raw: string;
  nodes: PlanNode[];
  warnings: string[];
}
export interface Analysis {
  statements: string[];
  warnings: string[];
  read_only: boolean;
}
export interface History {
  id: number;
  connection_id: string;
  sql: string;
  created_at: string;
  error: string | null;
  elapsed_ms: number;
}
interface Commands {
  connections: { args: undefined; result: Connection[] };
  save_connection: {
    args: {
      connection: Connection;
      password: string | null;
      remember: boolean;
    };
    result: Connection;
  };
  delete_connection: { args: { id: string }; result: void };
  connect: {
    args: { id: string; password: string | null };
    result: Capabilities;
  };
  test_connection: {
    args: { connection: Connection; password: string | null };
    result: Capabilities;
  };
  disconnect: { args: { id: string }; result: void };
  tables: { args: { id: string }; result: Table[] };
  inspect_table: { args: { id: string; table: Table }; result: TableInfo };
  table_select_sql: {
    args: { id: string; table: Table; limit: number };
    result: string;
  };
  transaction_state: {
    args: { id: string };
    result: "idle" | "active" | "failed";
  };
  apply_changes: {
    args: { id: string; table: Table; changes: Change[]; confirmed: boolean };
    result: MutationResult;
  };
  analyze_query: { args: { sql: string; engine: string }; result: Analysis };
  start_query: {
    args: {
      connection: string;
      sql: string;
      limit: number;
      timeoutSeconds: number;
      confirmed: boolean;
    };
    result: string;
  };
  query_status: { args: { id: string }; result: QueryStatus };
  start_plan: {
    args: {
      connection: string;
      sql: string;
      analyze: boolean;
      timeoutSeconds: number;
      confirmed: boolean;
    };
    result: string;
  };
  execution_plan: { args: { id: string }; result: ExecutionPlan };
  result_page: {
    args: { id: string; set: number; offset: number; limit: number };
    result: Row[];
  };
  cancel_query: { args: { id: string }; result: void };
  release_result: { args: { id: string }; result: void };
  load_document: { args: { id: string }; result: unknown };
  save_document: { args: { id: string; data: unknown }; result: void };
  history: { args: undefined; result: History[] };
  clear_history: { args: undefined; result: void };
  choose_database_file: { args: { create: boolean }; result: string | null };
  export_result: {
    args: { id: string; set: number; format: string; table: string };
    result: number | null;
  };
}
export function api<K extends keyof Commands>(
  command: K,
  ...args: Commands[K]["args"] extends undefined ? [] : [Commands[K]["args"]]
): Promise<Commands[K]["result"]> {
  return invoke(command, args[0]);
}
export function cellText(cell: Cell): string {
  if (cell.kind === "null") return "";
  if (cell.kind === "json") return JSON.stringify(cell.value);
  return String(cell.value);
}
