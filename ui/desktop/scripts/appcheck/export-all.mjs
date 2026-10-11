// Verify every app returned by the attached daemon, or the supplied ids.
// Usage: BIOROUTER_SERVER__SECRET_KEY=... node export-all.mjs <base> [ids...]
import { checkExport, exportHeaders } from './export-check-lib.mjs';

const base = process.argv[2] || 'http://127.0.0.1:3000';
let ids = process.argv.slice(3);
if (!ids.length) {
  const response = await fetch(new URL('/apps', base), { headers: exportHeaders() });
  if (!response.ok) throw new Error(`list apps http ${response.status}`);
  const rows = await response.json();
  ids = rows.filter((row) => !row.error).map((row) => row.id);
}
const results = [];
for (const id of ids) results.push(await checkExport(base, id));
console.log(`EXPORT: ${results.filter((result) => result.ok).length}/${ids.length} ok`);
for (const result of results) if (!result.ok) console.log(`  FAIL ${result.id}: ${result.err}`);
process.exitCode = results.every((result) => result.ok) ? 0 : 1;
