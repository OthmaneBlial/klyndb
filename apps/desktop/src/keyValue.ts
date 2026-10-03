import { cellText, type Cell, type KeyValue } from "./api";
export function keyLabel(key: Cell) {
  return key.kind === "binary" ? `0x${key.value}` : cellText(key) || '""';
}
export function ttlLabel(ttl: string) {
  return ttl === "-1"
    ? "No expiry"
    : ttl === "-2"
      ? "Missing key"
      : `${ttl} ms`;
}
export function formatKeyValue(value: KeyValue): string {
  if (value.kind === "array")
    return `[\n${value.value.map(formatKeyValue).join(",\n")}\n]`;
  const cell = value.value;
  if (cell.kind === "null") return "NULL";
  if (cell.kind === "binary") return `0x${cell.value}`;
  if (cell.kind === "number") return cell.value;
  return JSON.stringify(cell.value);
}
