import { lstat, readFile, readdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const siteRoot = fileURLToPath(new URL("../", import.meta.url));
const outputRoot = path.join(siteRoot, "dist-pages");
const canonicalUrl = "https://airwiki.github.io/airwiki/";
const socialImageUrl = `${canonicalUrl}assets/airwiki-social-preview.png`;
const expectedRootEntries = [
  "app.js",
  "assets",
  "index.html",
  "launch-config.js",
  "styles.css",
];
const expectedAssets = [
  "airwiki-demo-poster.png",
  "airwiki-demo.mp4",
  "airwiki-mark.png",
  "airwiki-review-flow.png",
  "airwiki-social-preview.png",
];
const allowedRemoteOrigins = new Set([
  "https://github.com",
  "https://docs.github.com",
]);
const scriptFiles = ["launch-config.js", "app.js"];
const activeFiles = [...scriptFiles, "styles.css"];

function assert(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

async function assertExactEntries(directory, expected) {
  const actual = (await readdir(directory)).sort();
  assert(
    JSON.stringify(actual) === JSON.stringify([...expected].sort()),
    `Unexpected Pages output in ${directory}: ${actual.join(", ")}`,
  );
}

async function assertRegularTree(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const entryPath = path.join(directory, entry.name);
    const metadata = await lstat(entryPath);
    assert(!metadata.isSymbolicLink(), `Pages output contains a symlink: ${entryPath}`);
    if (metadata.isDirectory()) {
      await assertRegularTree(entryPath);
    } else {
      assert(metadata.isFile(), `Pages output contains a non-regular entry: ${entryPath}`);
    }
  }
}

await assertExactEntries(outputRoot, expectedRootEntries);
await assertExactEntries(path.join(outputRoot, "assets"), expectedAssets);
await assertRegularTree(outputRoot);

const html = await readFile(path.join(outputRoot, "index.html"), "utf8");
assert(!html.includes("/__airwiki-protected"), "Pages HTML must not reference Worker-only routes");
assert(!/<form\b/i.test(html), "The public landing must not collect form data");
assert(!/<script\b(?![^>]*\bsrc=)/i.test(html), "Inline scripts are not permitted");
assert(
  html.includes("<meta http-equiv=\"Content-Security-Policy\""),
  "The defense-in-depth CSP meta element is missing",
);
assert(
  html.includes("<meta name=\"referrer\" content=\"no-referrer\">"),
  "The referrer policy meta element is missing",
);
assert(
  html.includes(`<link rel="canonical" href="${canonicalUrl}">`),
  "The canonical Pages URL is missing or incorrect",
);
assert(
  html.includes(`<meta property="og:url" content="${canonicalUrl}">`),
  "The Open Graph URL is missing or incorrect",
);
assert(
  html.includes(`<meta property="og:image" content="${socialImageUrl}">`),
  "The Open Graph image URL is missing or incorrect",
);
assert(
  html.includes(`<meta name="twitter:image" content="${socialImageUrl}">`),
  "The social-card image URL is missing or incorrect",
);
assert(
  html.includes("id=\"site-privacy\"") && html.includes("GitHub Pages"),
  "The host-specific privacy notice is missing",
);

const attributePattern = /\b(?:href|poster|src)="([^"]+)"/g;
for (const match of html.matchAll(attributePattern)) {
  const reference = match[1];
  if (reference.startsWith("#")) {
    continue;
  }

  const resolved = new URL(reference, canonicalUrl);
  if (resolved.origin !== "https://airwiki.github.io") {
    assert(
      allowedRemoteOrigins.has(resolved.origin),
      `Unreviewed remote origin in Pages HTML: ${resolved.origin}`,
    );
    continue;
  }

  assert(
    resolved.pathname.startsWith("/airwiki/"),
    `Local reference escapes the project-site base path: ${reference}`,
  );
  const relativePath = decodeURIComponent(resolved.pathname.slice("/airwiki/".length));
  if (relativePath === "") {
    continue;
  }
  const localPath = path.join(outputRoot, relativePath);
  assert(
    path.resolve(localPath).startsWith(`${path.resolve(outputRoot)}${path.sep}`),
    `Local Pages reference escapes the output directory: ${reference}`,
  );
  const metadata = await lstat(localPath);
  assert(metadata.isFile(), `Local Pages reference is not a file: ${reference}`);
}

for (const script of scriptFiles) {
  assert(
    html.includes(`<script src="${script}" type="module"></script>`),
    `Expected module script is missing: ${script}`,
  );
}

for (const filename of activeFiles) {
  const contents = await readFile(path.join(outputRoot, filename), "utf8");
  for (const match of contents.matchAll(/https?:\/\/[^\s"'`)]+/g)) {
    const remoteUrl = new URL(match[0]);
    assert(
      allowedRemoteOrigins.has(remoteUrl.origin),
      `Unreviewed remote origin in ${filename}: ${remoteUrl.origin}`,
    );
  }
}

for (const script of scriptFiles) {
  const contents = await readFile(path.join(outputRoot, script), "utf8");
  assert(
    !/\b(?:fetch|sendBeacon|WebSocket|XMLHttpRequest)\b/.test(contents),
    `The public landing script must not initiate network requests: ${script}`,
  );
}

console.log("Verified the static GitHub Pages launch artifact.");
