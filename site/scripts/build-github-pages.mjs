import { cp, lstat, mkdir, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const siteRoot = fileURLToPath(new URL("../", import.meta.url));
const repositoryRoot = fileURLToPath(new URL("../../", import.meta.url));
const outputRoot = path.join(siteRoot, "dist-pages");

const files = new Map([
  [path.join(siteRoot, "index.html"), "index.html"],
  [path.join(siteRoot, "styles.css"), "styles.css"],
  [path.join(siteRoot, "app.js"), "app.js"],
  [path.join(siteRoot, "launch-config.js"), "launch-config.js"],
  [path.join(siteRoot, "public/assets/airwiki-mark.png"), "assets/airwiki-mark.png"],
  [path.join(siteRoot, "public/assets/airwiki-review-flow.png"), "assets/airwiki-review-flow.png"],
  [path.join(siteRoot, "public/assets/airwiki-demo-poster.png"), "assets/airwiki-demo-poster.png"],
  [path.join(siteRoot, "public/assets/airwiki-demo.mp4"), "assets/airwiki-demo.mp4"],
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
