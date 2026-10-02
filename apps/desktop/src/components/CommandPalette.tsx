import { useState } from "react";
import { Search } from "lucide-react";
import { Modal } from "./Modal";
export function CommandPalette({
  commands,
  onClose,
}: {
  commands: { name: string; key: string; action: () => void | Promise<void> }[];
  onClose: () => void;
}) {
  const [paletteSearch, setPaletteSearch] = useState("");
  return (
    <Modal title="Jump to anything" onClose={onClose}>
      <div className="palette-search search">
        <Search size={18} />
        <input
          autoFocus
          aria-label="Search commands"
          value={paletteSearch}
          onChange={(e) => setPaletteSearch(e.target.value)}
          placeholder="Commands, connections, tables…"
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              commands
                .find((c) =>
                  c.name.toLowerCase().includes(paletteSearch.toLowerCase()),
                )
                ?.action();
              onClose();
            }
          }}
        />
      </div>
      <div className="command-list">
        {commands
          .filter((c) =>
            c.name.toLowerCase().includes(paletteSearch.toLowerCase()),
          )
          .slice(0, 100)
          .map((c) => (
            <button
              key={c.name}
              onClick={() => {
                c.action();
                onClose();
              }}
            >
              <span>{c.name}</span>
              <kbd>{c.key}</kbd>
            </button>
          ))}
      </div>
    </Modal>
  );
}
