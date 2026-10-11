// Verify the legacy text export's raw entry and standalone source build.
// Usage: BIOROUTER_SERVER__SECRET_KEY=... node export-check.mjs <base> <id>
import { checkExport } from './export-check-lib.mjs';

const result = await checkExport(process.argv[2], process.argv[3]);
console.log(JSON.stringify(result));
process.exitCode = result.ok ? 0 : 1;
