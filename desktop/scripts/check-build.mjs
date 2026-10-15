import { readdir, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const assetsRoot = path.join(desktopRoot, "dist", "assets");
const assetNames = await readdir(assetsRoot);
const javascriptAssets = assetNames.filter((name) => name.endsWith(".js"));
const fontAssets = assetNames.filter((name) => /\.(woff2?|ttf|otf)$/.test(name));

if (javascriptAssets.length < 2) {
  throw new Error(`Expected route splitting to produce multiple JavaScript assets; found ${javascriptAssets.length}`);
}

const unexpectedJakartaWeight = fontAssets.find((name) => name.includes("plus-jakarta-sans") && name.includes("-700-"));
if (unexpectedJakartaWeight) {
  throw new Error(`Unused Plus Jakarta Sans 700 asset was emitted: ${unexpectedJakartaWeight}`);
}

let javascriptBytes = 0;
for (const name of javascriptAssets) {
  javascriptBytes += (await stat(path.join(assetsRoot, name))).size;
}

console.log(`Build check passed: ${javascriptAssets.length} JS assets, ${fontAssets.length} font assets, ${javascriptBytes} JS bytes`);
