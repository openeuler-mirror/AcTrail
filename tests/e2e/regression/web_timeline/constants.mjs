// Load this case's Python dataclass once per Node.js process.
import { execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const { stdout } = await promisify(execFile)(
  'python3', [fileURLToPath(new URL('./constants.py', import.meta.url))],
);
export const C = Object.freeze(JSON.parse(stdout));
