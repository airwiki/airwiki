import { cp, lstat, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const siteRoot = fileURLToPath(new URL("../", import.meta.url));
const repositoryRoot = fileURLToPath(new URL("../../", import.meta.url));
const outputRoot = path.join(siteRoot, "dist-pages");
const sourceHtmlPath = path.join(siteRoot, "index.html");

const files = new Map([
  [path.join(siteRoot, "styles.css"), "styles.css"],
  [path.join(siteRoot, "app.js"), "app.js"],
  [path.join(siteRoot, "launch-config.js"), "launch-config.js"],
  [path.join(siteRoot, "src/assets/airwiki-mark.png"), "assets/airwiki-mark.png"],
  [path.join(siteRoot, "src/assets/airwiki-review-flow.png"), "assets/airwiki-review-flow.png"],
  [path.join(siteRoot, "src/assets/airwiki-demo-poster.png"), "assets/airwiki-demo-poster.png"],
  [path.join(siteRoot, "src/assets/airwiki-demo.mp4"), "assets/airwiki-demo.mp4"],
  [path.join(repositoryRoot, "resources/branding/github-social-preview.png"), "assets/airwiki-social-preview.png"],
]);

await rm(outputRoot, { recursive: true, force: true });
await mkdir(path.join(outputRoot, "assets"), { recursive: true, mode: 0o755 });

for (const [source, destination] of files) {
  const metadata = await lstat(source);
  if (!metadata.isFile() || metadata.isSymbolicLink()) {
    throw new Error(`Refusing non-regular Pages input: ${source}`);
  }

  await cp(source, path.join(outputRoot, destination), {
    dereference: false,
    errorOnExist: true,
    force: false,
  });
}

const sourceHtmlMetadata = await lstat(sourceHtmlPath);
if (!sourceHtmlMetadata.isFile() || sourceHtmlMetadata.isSymbolicLink()) {
  throw new Error(`Refusing non-regular Pages input: ${sourceHtmlPath}`);
}

const sourceHtml = await readFile(sourceHtmlPath, "utf8");
if (!sourceHtml.includes(" vite-ignore") || !sourceHtml.includes("/__airwiki-protected/")) {
  throw new Error("Pages source HTML is missing the expected Worker-route boundary");
}

const pagesHtml = sourceHtml
  .replaceAll(" vite-ignore", "")
  .replaceAll("/__airwiki-protected/", "");
await writeFile(path.join(outputRoot, "index.html"), pagesHtml, {
  encoding: "utf8",
  flag: "wx",
  mode: 0o644,
});
