import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

// Exercise the actual public snippet-copy handler without browser dependencies.
let copy;
let written;
const button = {
  dataset: { copy: 'install' },
  textContent: 'Copy commands',
  addEventListener: (_, handler) => { copy = handler; },
};
const code = { textContent: ' npm run tauri dev\n' };
const status = { textContent: '' };
runInNewContext(readFileSync(new URL('../site/app.js', import.meta.url), 'utf8'), {
  document: {
    querySelectorAll: () => [button],
    getElementById: (id) => id === 'install' ? code : status,
  },
  navigator: { clipboard: { writeText: async (text) => { written = text; } } },
  setTimeout: () => {},
});
await copy();
assert.equal(written, 'npm run tauri dev');
assert.equal(button.textContent, 'Copied ✓');
assert.equal(status.textContent, 'Commands copied to clipboard.');
console.log('Site snippet copy passed');
