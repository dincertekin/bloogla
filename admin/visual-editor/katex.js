// Copies KaTeX (drawing formulas on public pages) into ../static/katex
// (run by `npm run build`). Only the WOFF2 fonts are kept: every browser
// Bloogla supports reads them, and the program stays smaller.
const fs = require("fs");
const from = "node_modules/katex/dist";
const to = "../static/katex";
const version = require("./node_modules/katex/package.json").version;

fs.rmSync(to, { recursive: true, force: true });
fs.mkdirSync(`${to}/fonts`, { recursive: true });
const header = `/*! KaTeX ${version}, MIT licensed: https://github.com/KaTeX/KaTeX */\n`;
fs.writeFileSync(`${to}/katex.min.js`, header + fs.readFileSync(`${from}/katex.min.js`, "utf8"));
// The "swap" version shows formulas in a fallback font while KaTeX's own
// fonts load, rather than hiding them.
const css = fs
    .readFileSync(`${from}/katex-swap.min.css`, "utf8")
    .replace(/,url\(fonts\/[^)]+\.woff\) format\("woff"\),url\(fonts\/[^)]+\.ttf\) format\("truetype"\)/g, "");
if (css.includes(".woff)") || css.includes(".ttf)")) throw new Error("KaTeX's font rules changed: update katex.js");
fs.writeFileSync(`${to}/katex.min.css`, header + css);
for (const font of fs.readdirSync(`${from}/fonts`).filter((name) => name.endsWith(".woff2"))) {
    fs.copyFileSync(`${from}/fonts/${font}`, `${to}/fonts/${font}`);
}
