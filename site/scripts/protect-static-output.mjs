import { existsSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";

const siteRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const clientRoot = join(siteRoot, "dist", "client");
const assetRoot = join(clientRoot, "assets");
const generatedEntryPattern = /^index-[A-Za-z0-9_-]+\.(?:css|js)$/;

const indexPath = join(clientRoot, "index.html");
if (!existsSync(indexPath)) {
  throw new Error("Vite did not generate the expected client index.html");
}
rmSync(indexPath);

if (existsSync(assetRoot)) {
  for (const entry of readdirSync(assetRoot, { withFileTypes: true })) {
    if (entry.isFile() && generatedEntryPattern.test(entry.name)) {
      rmSync(join(assetRoot, entry.name));
      continue;
    }
    throw new Error(`Unexpected generated client asset: ${entry.name}`);
  }

  if (readdirSync(assetRoot).length !== 0) {
    throw new Error("Generated client assets remain after active entries were removed");
  }
}

const expectedClientEntries = new Set([".assetsignore", "_headers", "assets"]);
for (const entry of readdirSync(clientRoot)) {
  if (!expectedClientEntries.has(entry)) {
    throw new Error(`Unexpected generated client entry: ${entry}`);
  }
  rmSync(join(clientRoot, entry), { recursive: true });
}

if (readdirSync(clientRoot).length !== 0) {
  throw new Error("Static client files remain after Worker-only protection");
}

const serverBundle = join(siteRoot, "dist", "server", "index.js");
const compressedBundleBytes = gzipSync(readFileSync(serverBundle)).byteLength;
const compressedBundleBudget = 2_400_000;
if (compressedBundleBytes > compressedBundleBudget) {
  throw new Error(
    `Compressed Worker bundle exceeds ${compressedBundleBudget} bytes: ${compressedBundleBytes}`,
  );
}
