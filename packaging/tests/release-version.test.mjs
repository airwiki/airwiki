import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { readWorkspaceVersion } from "../release-version.mjs";

async function workspace(cargo, tauri, frontend) {
  const root = await mkdtemp(join(tmpdir(), "airwiki-release-version-"));
  await mkdir(join(root, "apps/desktop/ui"), { recursive: true });
  await writeFile(join(root, "Cargo.toml"), cargo, "utf8");
  await writeFile(join(root, "apps/desktop/tauri.conf.json"), JSON.stringify(tauri), "utf8");
  await writeFile(join(root, "apps/desktop/ui/package.json"), JSON.stringify(frontend), "utf8");
  return root;
}

test("matching stable versions are accepted", async () => {
  const root = await workspace(
    '[workspace.package]\nversion = "1.2.3"\n',
    { version: "1.2.3" },
    { version: "1.2.3" },
  );

  assert.equal(readWorkspaceVersion(root), "1.2.3");
});

test("mismatched versions are rejected", async () => {
  const root = await workspace(
    '[workspace.package]\nversion = "1.2.3"\n',
    { version: "1.2.4" },
    { version: "1.2.3" },
  );

  assert.throws(() => readWorkspaceVersion(root), /do not match/);
});

test("prerelease versions are rejected", async () => {
  const root = await workspace(
    '[workspace.package]\nversion = "1.2.3-rc.1"\n',
    { version: "1.2.3-rc.1" },
    { version: "1.2.3-rc.1" },
  );

  assert.throws(() => readWorkspaceVersion(root), /stable three-part semver/);
});
