// Builds src/adblock-resources.json (uBlock Origin scriptlets for the adblock
// engine), the same way Brave does in brave-core-crx-packager.
// Usage: git clone --depth 1 https://github.com/gorhill/uBlock /tmp/ubo
//        bun scripts/gen-adblock-resources.mjs /tmp/ubo
import { writeFileSync } from "fs";
import { execSync } from "child_process";
const ubo = process.argv[2] ?? "/tmp/ubo";
const { builtinScriptlets } = await import(`${ubo}/src/js/resources/scriptlets.js`);
const names = new Set(builtinScriptlets.map((s) => s.name));
const out = builtinScriptlets.map((s) => {
  for (const d of s.dependencies ?? []) if (!names.has(d)) throw new Error(`${s.name}: missing dependency ${d}`);
  return {
    name: s.name,
    aliases: s.aliases ?? [],
    kind: { mime: "application/javascript" },
    content: Buffer.from(s.fn.toString()).toString("base64"),
    dependencies: s.dependencies ?? [],
  };
});
const rev = execSync(`git -C ${ubo} log -1 --format=%h`).toString().trim();
writeFileSync("src/adblock-resources.json", JSON.stringify(out));
console.log(`${out.length} scriptlets from uBlock ${rev}`);
