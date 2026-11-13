// Rebuilds dist/app.js from src/*.ts with no bundler installed:
// Node's built-in TypeScript stripping (node:module), one function scope per
// module, imports resolved by name. Agent Drafter's own fallback bundle
// (used when esbuild is missing) dropped prevent.ts and preventData.ts and
// mangled object literals, so the app must be rebuilt with this script.
//   node build.mjs
import { stripTypeScriptTypes } from "node:module";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";

const ORDER = ["sdk", "preventData", "prevent", "cohort", "main"];
const IMPORT = /^import\s+(?:type\s+)?\{([^}]*)\}\s+from\s+"\.\/(\w+)";?[ \t]*$/gm;
const EXPORT_DECL = /^export\s+((?:async\s+)?(?:function\*?|const|let|var|class)\s+([A-Za-z_$][\w$]*))/gm;

const modules = ORDER.map((name) => {
  let code = stripTypeScriptTypes(readFileSync(`src/${name}.ts`, "utf8"), { mode: "strip" });
  const imports = [];
  code = code.replace(IMPORT, (_, names, from) => {
    if (!ORDER.includes(from)) throw new Error(`${name}.ts imports unknown module ./${from}`);
    const list = names.split(",").map((s) => s.trim()).filter((s) => s && !s.startsWith("type "));
    if (list.length) imports.push(`const { ${list.map((s) => s.replace(/\s+as\s+/, ": ")).join(", ")} } = __mod.${from};`);
    return "";
  });
  const exported = [];
  code = code.replace(EXPORT_DECL, (_, decl, id) => { exported.push(id); return decl; });
  if (/^\s*(import|export)\s/m.test(code)) throw new Error(`${name}.ts has an import or export form this build does not handle`);
  return `// ---- src/${name}.ts\n__mod.${name} = (function () {\n${imports.join("\n")}\n${code}\nreturn { ${exported.join(", ")} };\n})();`;
});

mkdirSync("dist", { recursive: true });
writeFileSync("dist/app.js",
  "/* PREVENT UCSF Risk Calculator. Built by build.mjs from src/ (Node TypeScript stripping). */\n" +
  "(function () {\nconst __mod = {};\n" + modules.join("\n\n") + "\n})();\n");
console.log("dist/app.js written");
