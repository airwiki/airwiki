import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import worker from "../dist/server/index.js";

const siteRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const securityHeaders = new Map([
  [
    "content-security-policy",
    "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; media-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'",
  ],
  ["cross-origin-opener-policy", "same-origin"],
  ["cross-origin-resource-policy", "same-origin"],
  [
    "permissions-policy",
    "accelerometer=(), ambient-light-sensor=(), autoplay=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()",
  ],
  ["referrer-policy", "no-referrer"],
  ["x-content-type-options", "nosniff"],
  ["x-frame-options", "DENY"],
]);
const protectedPrefix = "/__airwiki-protected/";
const protectedAssets = new Map([
  ["airwiki-demo-poster.png", "image/png"],
  ["airwiki-demo.mp4", "video/mp4"],
  ["airwiki-mark.png", "image/png"],
  ["airwiki-review-flow.png", "image/png"],
]);

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function assertSecurityHeaders(response, context) {
  for (const [name, expected] of securityHeaders) {
    assert(response.headers.get(name) === expected, `${context} has invalid ${name}`);
  }
}

async function fetchWorker(path, init) {
  return worker.fetch(new Request(`https://preview.invalid${path}`, init), {}, {});
}

function sha256(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

const clientRoot = join(siteRoot, "dist", "client");
assert(existsSync(clientRoot), "empty client root must exist for the local Worker binding");
assert(readdirSync(clientRoot).length === 0, "static client output must be empty");

const root = await fetchWorker("/");
assert(root.status === 200, `root returned ${root.status}`);
assertSecurityHeaders(root, "root");
const html = await root.text();
const references = [...html.matchAll(/\b(href|poster|src|srcset)=(["'])(.*?)\2/g)]
  .flatMap(([, attribute, , value]) => attribute === "srcset"
    ? value.split(",").map((candidate) => candidate.trim().split(/\s+/, 1)[0])
    : [value])
  .filter((value) => !value.startsWith("#") && !/^https:\/\//.test(value));
assert(references.length > 0, "root contains no local references");
for (const reference of references) {
  assert(reference.startsWith(protectedPrefix), `unprotected local reference: ${reference}`);
  const response = await fetchWorker(reference);
  assert(response.status === 200, `${reference} returned ${response.status}`);
  assertSecurityHeaders(response, reference);

  const headResponse = await fetchWorker(reference, { method: "HEAD" });
  assert(headResponse.status === 200, `HEAD ${reference} returned ${headResponse.status}`);
  assertSecurityHeaders(headResponse, `HEAD ${reference}`);
  assert((await headResponse.text()) === "", `HEAD ${reference} returned a body`);
}

const index = await fetchWorker("/index.html");
assert(index.status === 200, `/index.html returned ${index.status}`);
assertSecurityHeaders(index, "/index.html");
assert((await index.text()) === html, "/index.html differs from root");

const cssResponse = await fetchWorker("/__airwiki-protected/styles.css");
const css = await cssResponse.text();
assert(!/url\s*\(/i.test(css), "stylesheet contains an unverified url() reference");

for (const [assetName, contentType] of protectedAssets) {
  const route = `${protectedPrefix}assets/${assetName}`;
  const sourcePath = join(siteRoot, "src", "assets", assetName);
  const source = readFileSync(sourcePath);
  const response = await fetchWorker(route);
  const bytes = new Uint8Array(await response.arrayBuffer());
  assert(response.status === 200, `${route} returned ${response.status}`);
  assert(response.headers.get("content-type") === contentType, `${route} has invalid type`);
  assert(
    response.headers.get("content-length") === String(statSync(sourcePath).size),
    `${route} has invalid length`,
  );
  assert(sha256(bytes) === sha256(source), `${route} differs from its bundled source`);
  assertSecurityHeaders(response, route);
}

const head = await fetchWorker("/", { method: "HEAD" });
assert(head.status === 200, `HEAD root returned ${head.status}`);
assertSecurityHeaders(head, "HEAD root");
assert((await head.text()) === "", "HEAD root returned a body");

const missing = await fetchWorker("/missing");
assert(missing.status === 404, `missing route returned ${missing.status}`);
assertSecurityHeaders(missing, "missing route");

const method = await fetchWorker("/", { method: "POST" });
assert(method.status === 405, `POST root returned ${method.status}`);
assert(method.headers.get("allow") === "GET, HEAD", "POST root has invalid Allow header");
assertSecurityHeaders(method, "POST root");

const options = await fetchWorker("/", { method: "OPTIONS" });
assert(options.status === 405, `OPTIONS root returned ${options.status}`);
assert(options.headers.get("allow") === "GET, HEAD", "OPTIONS root has invalid Allow header");
assertSecurityHeaders(options, "OPTIONS root");

for (const directPath of [
  "/assets/airwiki-demo-poster.png",
  "/assets/airwiki-demo.mp4",
  "/assets/airwiki-mark.png",
  "/assets/airwiki-review-flow.png",
  "/_headers",
  "/.assetsignore",
]) {
  const response = await fetchWorker(directPath);
  assert(response.status === 404, `${directPath} returned ${response.status}`);
  assertSecurityHeaders(response, directPath);
}

const videoSize = statSync(join(siteRoot, "src", "assets", "airwiki-demo.mp4")).size;
const range = await fetchWorker("/__airwiki-protected/assets/airwiki-demo.mp4", {
  headers: { Range: "bytes=2-5" },
});
assert(range.status === 206, `video range returned ${range.status}`);
assert(range.headers.get("content-range") === `bytes 2-5/${videoSize}`, "video range is invalid");
assert(range.headers.get("content-length") === "4", "video range length is invalid");
assertSecurityHeaders(range, "video range");
assert((await range.arrayBuffer()).byteLength === 4, "video range body is invalid");

const suffixRange = await fetchWorker("/__airwiki-protected/assets/airwiki-demo.mp4", {
  headers: { Range: "bytes=-3" },
});
assert(suffixRange.status === 206, `suffix video range returned ${suffixRange.status}`);
assert(
  suffixRange.headers.get("content-range") === `bytes ${videoSize - 3}-${videoSize - 1}/${videoSize}`,
  "suffix video range is invalid",
);
assert((await suffixRange.arrayBuffer()).byteLength === 3, "suffix range body is invalid");

const unsatisfiableRange = await fetchWorker(
  "/__airwiki-protected/assets/airwiki-demo.mp4",
  { headers: { Range: `bytes=${videoSize}-` } },
);
assert(unsatisfiableRange.status === 416, `unsatisfiable range returned ${unsatisfiableRange.status}`);
assert(
  unsatisfiableRange.headers.get("content-range") === `bytes */${videoSize}`,
  "unsatisfiable range is invalid",
);
assertSecurityHeaders(unsatisfiableRange, "unsatisfiable range");

for (const rangeHeader of ["bytes=0-1,3-4", "items=0-3"]) {
  const response = await fetchWorker("/__airwiki-protected/assets/airwiki-demo.mp4", {
    headers: { Range: rangeHeader },
  });
  assert(response.status === 200, `${rangeHeader} returned ${response.status}`);
  assert(response.headers.get("content-range") === null, `${rangeHeader} invented a range`);
  assert((await response.arrayBuffer()).byteLength === videoSize, `${rangeHeader} lost content`);
  assertSecurityHeaders(response, rangeHeader);
}

const headRange = await fetchWorker("/__airwiki-protected/assets/airwiki-demo.mp4", {
  method: "HEAD",
  headers: { Range: "bytes=2-5" },
});
assert(headRange.status === 206, `HEAD video range returned ${headRange.status}`);
assert(
  headRange.headers.get("content-range") === `bytes 2-5/${videoSize}`,
  "HEAD video range is invalid",
);
assert((await headRange.text()) === "", "HEAD video range returned a body");
assertSecurityHeaders(headRange, "HEAD video range");
