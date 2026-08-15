import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { compareStableVersions, readWorkspaceVersion } from "../release-version.mjs";

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

test("mismatched AirWiki path dependencies are rejected", async () => {
  const root = await workspace(
    '[workspace.package]\nversion = "1.2.3"\n',
    { version: "1.2.3" },
    { version: "1.2.3" },
  );
  await mkdir(join(root, "crates/example"), { recursive: true });
  await writeFile(
    join(root, "crates/example/Cargo.toml"),
    '[package]\nname = "airwiki-example"\nversion.workspace = true\n\n[dependencies]\n' +
      'airwiki-types = { version = "1.2.2", path = "../airwiki-types" }\n',
    "utf8",
  );

  assert.throws(() => readWorkspaceVersion(root), /path-dependency versions do not match/);
});

test("stable versions compare numerically without precision loss", () => {
  assert.equal(compareStableVersions("1.10.0", "1.9.99"), 1);
  assert.equal(compareStableVersions("2.0.0", "2.0.0"), 0);
  assert.equal(compareStableVersions("99999999999999999999.0.0", "9.999.999"), 1);
  assert.equal(compareStableVersions("0.9.9", "1.0.0"), -1);
});

test("release comparison rejects prerelease and malformed versions", () => {
  assert.throws(
    () => compareStableVersions("1.2.3-rc.1", "1.2.2"),
    /stable three-part semver/,
  );
  assert.throws(() => compareStableVersions("1.2", "1.1.0"), /stable three-part semver/);
});
