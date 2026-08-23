#!/usr/bin/env node

import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const STABLE_SEMVER = /^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)$/;

export function compareStableVersions(left, right) {
  if (!STABLE_SEMVER.test(left) || !STABLE_SEMVER.test(right)) {
    throw new Error("release comparison requires stable three-part semver values");
  }
  const leftParts = left.split(".").map(BigInt);
  const rightParts = right.split(".").map(BigInt);
  for (let index = 0; index < leftParts.length; index += 1) {
    if (leftParts[index] > rightParts[index]) {
      return 1;
    }
    if (leftParts[index] < rightParts[index]) {
      return -1;
    }
  }
  return 0;
}

function cargoWorkspaceVersion(source) {
  let inWorkspacePackage = false;
  for (const line of source.split(/\r?\n/)) {
    if (line === "[workspace.package]") {
      inWorkspacePackage = true;
      continue;
    }
    if (inWorkspacePackage && line.startsWith("[")) {
      break;
    }
    if (inWorkspacePackage) {
      const version = line.match(/^version\s*=\s*"([^"]+)"\s*$/)?.[1];
      if (typeof version === "string") {
        return version;
      }
    }
  }
  throw new Error("release version is missing from Cargo.toml [workspace.package]");
}

function cargoManifests(root) {
  const manifests = [];
  const visit = (directory) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      if (entry.isSymbolicLink()) {
        continue;
      }
      const path = resolve(directory, entry.name);
      if (entry.isDirectory()) {
        if (![".git", "node_modules", "target"].includes(entry.name)) {
          visit(path);
        }
      } else if (entry.isFile() && entry.name === "Cargo.toml") {
        manifests.push(path);
      }
    }
  };
  visit(root);
  return manifests.sort();
}

function validateCargoPathDependencyVersions(root, version) {
  const mismatches = [];
  for (const manifest of cargoManifests(root)) {
    for (const line of readFileSync(manifest, "utf8").split(/\r?\n/)) {
      const dependency = line.match(/^\s*(airwiki-[a-z0-9-]+)\s*=\s*\{([^}]*)\}\s*$/);
      if (dependency === null || !/\bpath\s*=/.test(dependency[2])) {
        continue;
      }
      const declared = dependency[2].match(/\bversion\s*=\s*"([^"]+)"/)?.[1];
      if (declared !== version) {
        const relative = manifest.slice(root.length + 1);
        mismatches.push(`${relative}:${dependency[1]}=${declared ?? "missing"}`);
      }
    }
  }
  if (mismatches.length > 0) {
    throw new Error(
      `AirWiki path-dependency versions do not match ${version}: ${mismatches.join(", ")}`,
    );
  }
}

export function readWorkspaceVersion(root) {
  const versions = new Map([
    ["Cargo.toml", cargoWorkspaceVersion(readFileSync(resolve(root, "Cargo.toml"), "utf8"))],
    [
      "apps/desktop/tauri.conf.json",
      JSON.parse(readFileSync(resolve(root, "apps/desktop/tauri.conf.json"), "utf8")).version,
    ],
    [
      "apps/desktop/ui/package.json",
      JSON.parse(readFileSync(resolve(root, "apps/desktop/ui/package.json"), "utf8")).version,
    ],
  ]);
  for (const [path, version] of versions) {
    if (typeof version !== "string") {
      throw new Error(`release version is missing or is not a string in ${path}`);
    }
  }
  if (new Set(versions.values()).size !== 1) {
    const detail = [...versions].map(([path, version]) => `${path}=${version}`).join(", ");
    throw new Error(`release versions do not match: ${detail}`);
  }
  const version = versions.values().next().value;
  if (!STABLE_SEMVER.test(version)) {
    throw new Error("release version must be a stable three-part semver");
  }
  validateCargoPathDependencyVersions(root, version);
  return version;
}

function argument(name) {
  const index = process.argv.indexOf(name);
  if (index === -1) {
    return undefined;
  }
  const value = process.argv[index + 1];
  if (typeof value !== "string" || value.startsWith("--")) {
    throw new Error(`${name} requires a value`);
  }
  return value;
}

function main() {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const version = readWorkspaceVersion(root);
  const expected = argument("--expect");
  const tag = argument("--tag");
  const newerThan = argument("--newer-than");
  if (expected !== undefined && expected !== version) {
    throw new Error(`requested version ${expected} does not match ${version}`);
  }
  if (tag !== undefined && tag !== `v${version}`) {
    throw new Error(`requested tag ${tag} does not match v${version}`);
  }
  if (newerThan !== undefined && compareStableVersions(version, newerThan) <= 0) {
    throw new Error(`requested version ${version} is not newer than ${newerThan}`);
  }
  process.stdout.write(`${version}\n`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    const message = error instanceof Error ? error.message : "release version validation failed";
    process.stderr.write(`${message}\n`);
    process.exitCode = 1;
  }
}
