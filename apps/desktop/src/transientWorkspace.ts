// Native query text and data never enter the persisted SQL workspace.
export class TransientWorkspaceCache<T> {
  private entries = new Map<
    string,
    { state: T; bytes: number; clear: (state: T, reason: string) => T }
  >();

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

  invalidate(id: string) {
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
  }
}
