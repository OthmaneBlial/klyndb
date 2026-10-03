import type { DocumentWorkspaceState } from "./components/DocumentWorkspace";
import type { KeyWorkspaceState } from "./components/KeyValueWorkspace";

export function documentResultBytes({
  databases,
  collections,
  page,
  indexes,
}: Pick<
  DocumentWorkspaceState,
  "databases" | "collections" | "page" | "indexes"
>) {
  return JSON.stringify({ databases, collections, page, indexes }).length * 2;
}
export function keyResultBytes({
  page,
  inspection,
  reply,
  positions,
}: Pick<KeyWorkspaceState, "page" | "inspection" | "reply" | "positions">) {
  return JSON.stringify({ page, inspection, reply, positions }).length * 2;
}

// Native query text and data never enter the persisted SQL workspace.
export class TransientWorkspaceCache<T> {
  private entries = new Map<
    string,
    { state: T; bytes: number; clear: (state: T, reason: string) => T }
  >();

  private epochs = new Map<string, object>();

  constructor(private budget = 32 * 1024 * 1024) {}

  restore(id: string): T | undefined {
    const entry = this.entries.get(id);
    if (entry) {
      this.entries.delete(id);
      this.entries.set(id, entry);
    }
    return entry?.state;
  }

  remember(
    id: string,
    state: T,
    bytes: number,
    clear: (state: T, reason: string) => T,
  ) {
    this.entries.delete(id);
    this.entries.set(id, { state, bytes, clear });
    let retained = [...this.entries.values()].reduce((n, e) => n + e.bytes, 0);
    for (const entry of this.entries.values()) {
      if (retained <= this.budget) break;
      if (!entry.bytes) continue;
      retained -= entry.bytes;
      entry.state = entry.clear(
        entry.state,
        "Cached results were cleared to limit memory. Your query draft is kept; run it again to refresh.",
      );
      entry.bytes = 0;
    }
  }

  // Replies may outlive the mounted tab, but never its connection session.
  background(id: string, measure: (state: T) => number, changed: () => void) {
    const epoch = this.epochs.get(id) ?? {};
    this.epochs.set(id, epoch);
    return (patch: Partial<T>) => {
      const entry = this.entries.get(id);
      if (!entry || this.epochs.get(id) !== epoch) return;
      const state = { ...entry.state, ...patch };
      this.remember(id, state, measure(state), entry.clear);
      changed();
    };
  }

  invalidate(id: string) {
    this.epochs.set(id, {});
    const entry = this.entries.get(id);
    if (entry) {
      entry.state = entry.clear(
        entry.state,
        "The connection session changed. Your draft is kept; reload the catalog and results before editing.",
      );
      entry.bytes = 0;
    }
  }

  forget(id: string) {
    this.entries.delete(id);
    this.epochs.delete(id);
  }
}
