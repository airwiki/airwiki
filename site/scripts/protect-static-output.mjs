import { existsSync, readdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const siteRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const clientRoot = join(siteRoot, "dist", "client");
const assetRoot = join(clientRoot, "assets");
const expectedAssets = new Set([
  "airwiki-demo-poster.png",
  "airwiki-demo.mp4",
  "airwiki-mark.png",
  "airwiki-review-flow.png",
]);
const generatedEntryPattern = /^index-[A-Za-z0-9_-]+\.(?:css|js)$/;

const indexPath = join(clientRoot, "index.html");
if (!existsSync(indexPath)) {
  throw new Error("Vite did not generate the expected client index.html");
}
rmSync(indexPath);

for (const entry of readdirSync(assetRoot, { withFileTypes: true })) {
  if (entry.isFile() && expectedAssets.has(entry.name)) continue;
  if (entry.isFile() && generatedEntryPattern.test(entry.name)) {
    rmSync(join(assetRoot, entry.name));
    continue;
  }
  throw new Error(`Unexpected generated client asset: ${entry.name}`);
}

for (const assetName of expectedAssets) {
  if (!existsSync(join(assetRoot, assetName))) {
    throw new Error(`Missing protected backing asset: ${assetName}`);
  }
}

const expectedClientEntries = new Set([".assetsignore", "_headers", "assets"]);
for (const entry of readdirSync(clientRoot)) {
  if (!expectedClientEntries.has(entry)) {
    throw new Error(`Unexpected generated client entry: ${entry}`);
  }
}
