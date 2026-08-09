import { createHash } from 'node:crypto';
import { existsSync, lstatSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const workspaceRoot = resolve(uiRoot, '../../..');
const targets = {
  'macos-arm64': { platform: 'darwin', arch: 'arm64', output: 'NPM_LICENSES_MACOS_ARM64.md' },
  'windows-x64': { platform: 'win32', arch: 'x64', output: 'NPM_LICENSES_WINDOWS_X64.md' }
};
const allowedLicenses = new Set([
  '(MIT AND Zlib)', '(MIT OR CC0-1.0)', '(MIT OR GPL-3.0-or-later)',
  '0BSD', 'Apache-2.0', 'Apache-2.0 OR MIT', 'BSD-2-Clause', 'BSD-3-Clause',
  'BlueOak-1.0.0', 'CC-BY-3.0', 'CC-BY-4.0', 'CC0-1.0', 'ISC', 'MIT',
  'MIT-0', 'MPL-2.0', 'Python-2.0', 'Unlicense'
]);
const reviewedMetadataOverrides = new Map([
  ['css-value@0.0.1', { reportedLicense: 'Unknown', license: 'MIT', legalFile: 'Readme.md' }]
]);
const [mode, targetName] = process.argv.slice(2);
const target = targets[targetName];

if (!['--check', '--generate'].includes(mode) || !target) {
  throw new Error('usage: node scripts/license-inventory.mjs --check|--generate macos-arm64|windows-x64');
}
if (process.platform !== target.platform || process.arch !== target.arch) {
  throw new Error(`${targetName} inventory requires ${target.platform}/${target.arch}`);
}

const packageManifest = JSON.parse(readFileSync(join(uiRoot, 'package.json'), 'utf8'));
if (packageManifest.packageManager !== 'pnpm@10.18.3') {
  throw new Error('license inventory requires pnpm@10.18.3');
}

const command = process.platform === 'win32'
  ? join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'cmd.exe')
  : 'pnpm';
const commandArguments = process.platform === 'win32'
  ? ['/d', '/s', '/c', 'pnpm.cmd licenses list --json --long']
  : ['licenses', 'list', '--json', '--long'];
const result = spawnSync(command, commandArguments, {
  cwd: uiRoot,
  encoding: 'utf8',
  maxBuffer: 16 * 1024 * 1024
});
if (result.status !== 0) {
  throw new Error(`pnpm license inspection failed: ${result.stderr.trim()}`);
}

const report = JSON.parse(result.stdout);
const packages = [];
const legalTexts = new Map();
const missingLegalTexts = [];
for (const [license, entries] of Object.entries(report)) {
  for (const entry of entries) {
    const packageKey = `${entry.name}@${[...entry.versions].sort().join(',')}`;
    const metadataOverride = reviewedMetadataOverrides.get(packageKey);
    if (metadataOverride && metadataOverride.reportedLicense !== license) {
      throw new Error(`stale npm license metadata override: ${packageKey}`);
    }
    if (!allowedLicenses.has(license) && !metadataOverride) {
      throw new Error(`unreviewed npm license expression: ${license} (${packageKey})`);
    }
    const hashes = new Set();
    for (const packagePath of entry.paths) {
      for (const legalPath of legalFiles(packagePath, metadataOverride?.legalFile)) {
        const content = normalizeLegalText(readFileSync(legalPath, 'utf8'));
        const hash = sha256(content);
        hashes.add(hash);
        const existing = legalTexts.get(hash);
        if (existing) {
          existing.origins.add(`${entry.name}@${entry.versions.join(', ')}`);
        } else {
          legalTexts.set(hash, {
            content,
            filename: basename(legalPath),
            origins: new Set([`${entry.name}@${entry.versions.join(', ')}`])
          });
        }
      }
    }
    if (hashes.size === 0) {
      missingLegalTexts.push(`${entry.name}@${entry.versions.join(', ')}`);
    }
    packages.push({
      name: entry.name,
      versions: [...entry.versions].sort(),
      license: metadataOverride?.license ?? license,
      homepage: entry.homepage ?? '',
      hashes: [...hashes].sort()
    });
  }
}
packages.sort((left, right) => compareText(left.name, right.name) || compareText(left.versions.join(), right.versions.join()));

const lockHash = sha256(readFileSync(join(uiRoot, 'pnpm-lock.yaml')));
const markdown = renderMarkdown(targetName, lockHash, packages, legalTexts, missingLegalTexts);
const output = join(workspaceRoot, 'resources', 'licenses', target.output);
if (mode === '--generate') {
  writeFileSync(output, markdown);
  process.stdout.write(`generated ${output}\n`);
} else {
  if (!existsSync(output) || readFileSync(output, 'utf8') !== markdown) {
    throw new Error(`${output} is stale; generate it on ${targetName} and review the diff`);
  }
  process.stdout.write(`${targetName} npm license inventory is current\n`);
}

function legalFiles(packagePath, reviewedLegalFile) {
  const metadata = lstatSync(packagePath);
  if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
    throw new Error(`package path is not a regular directory: ${packagePath}`);
  }
  const files = readdirSync(packagePath, { withFileTypes: true })
    .filter((entry) => entry.isFile() && /^(license|licence|copying|notice|copyright|unlicense)([._-].*)?$/i.test(entry.name))
    .map((entry) => join(packagePath, entry.name))
    .sort();
  if (reviewedLegalFile) {
    const reviewedPath = join(packagePath, reviewedLegalFile);
    const reviewedMetadata = lstatSync(reviewedPath);
    if (!reviewedMetadata.isFile() || reviewedMetadata.isSymbolicLink()) {
      throw new Error(`reviewed legal path is not a regular file: ${reviewedPath}`);
    }
    files.push(reviewedPath);
  }
  return files;
}

function normalizeLegalText(content) {
  return `${content.replaceAll('\r\n', '\n').replaceAll('\r', '\n').trimEnd()}\n`;
}

function sha256(content) {
  return createHash('sha256').update(content).digest('hex');
}

function renderMarkdown(targetName, lockHash, entries, texts, metadataOnly) {
  let output = '# Complete npm License Inventory\n\n';
  output += 'This file is generated. Do not edit it manually. Regenerate it with the pinned pnpm installation and `pnpm run licenses:generate`.\n\n';
  output += `Platform: \`${targetName}\`. Lockfile SHA-256: \`${lockHash}\`. Package manager: \`pnpm@10.18.3\`.\n\n`;
  output += 'The inventory covers the installed production and development dependency closure. Local paths and machine-specific state are excluded. Identical legal texts are stored once by normalized SHA-256. Packages whose published tarball omits a legal file remain listed with their declared license and are called out explicitly.\n\n';
  output += '## Packages\n\n| Package | Version(s) | Declared license | Homepage | Legal text SHA-256 |\n';
  output += '| --- | --- | --- | --- | --- |\n';
  for (const entry of entries) {
    const homepage = entry.homepage ? `[source](${entry.homepage})` : '';
    const hashes = entry.hashes.length > 0
      ? entry.hashes.map((hash) => `\`${hash}\``).join('<br>')
      : 'declared metadata only';
    output += `| ${escapeCell(entry.name)} | ${escapeCell(entry.versions.join(', '))} | ${escapeCell(entry.license)} | ${homepage} | ${hashes} |\n`;
  }
  output += `\n## Packages without a published legal file\n\n${metadataOnly.sort().map((entry) => `- \`${entry}\``).join('\n')}\n`;
  output += '\n## Deduplicated legal texts\n';
  for (const [hash, text] of [...texts.entries()].sort(([left], [right]) => compareText(left, right))) {
    output += `\n### ${hash}\n\nOrigins: ${[...text.origins].sort().map((origin) => `\`${origin}\``).join(', ')}. Upstream filename: \`${text.filename}\`.\n\n~~~~text\n${text.content}~~~~\n`;
  }
  return output;
}

function escapeCell(value) {
  return String(value).replaceAll('|', '\\|').replaceAll('\n', ' ');
}

function compareText(left, right) {
  return left < right ? -1 : left > right ? 1 : 0;
}
