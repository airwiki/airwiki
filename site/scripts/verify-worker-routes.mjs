import worker from "../dist/server/index.js";

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
const assetBodies = new Map([
  ["/assets/airwiki-demo-poster.png", new Uint8Array([1, 2, 3])],
  ["/assets/airwiki-demo.mp4", new Uint8Array([0, 1, 2, 3, 4, 5, 6, 7, 8, 9])],
  ["/assets/airwiki-mark.png", new Uint8Array([1, 2, 3])],
  ["/assets/airwiki-review-flow.png", new Uint8Array([1, 2, 3])],
]);
const env = {
  ASSETS: {
    async fetch(request) {
      const url = new URL(request.url);
      const rangeHeader = request.headers.get("Range");
      assert(
        rangeHeader === null || /^bytes=(?:\d+-\d*|-\d+)$/.test(rangeHeader),
        "unsupported Range reached the backing asset service",
      );
      const body = assetBodies.get(url.pathname);
      if (!body) return new Response("Not found\n", { status: 404 });
      return new Response(request.method === "HEAD" ? null : body, {
        headers: {
          "Content-Length": String(body.byteLength),
          "Content-Type": url.pathname.endsWith(".mp4") ? "video/mp4" : "image/png",
        },
      });
    },
  },
};

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function assertSecurityHeaders(response, context) {
  for (const [name, expected] of securityHeaders) {
    assert(response.headers.get(name) === expected, `${context} has invalid ${name}`);
  }
}

async function fetchWorker(path, init) {
  return worker.fetch(new Request(`https://preview.invalid${path}`, init), env, {});
}

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

const directBacking = await fetchWorker("/assets/airwiki-demo.mp4");
assert(directBacking.status === 404, `direct backing route returned ${directBacking.status}`);
assertSecurityHeaders(directBacking, "direct backing route");

const range = await fetchWorker("/__airwiki-protected/assets/airwiki-demo.mp4", {
  headers: { Range: "bytes=2-5" },
});
assert(range.status === 206, `video range returned ${range.status}`);
assert(range.headers.get("content-range") === "bytes 2-5/10", "video range is invalid");
assert(range.headers.get("content-length") === "4", "video range length is invalid");
assertSecurityHeaders(range, "video range");
const rangeBody = new Uint8Array(await range.arrayBuffer());
assert(rangeBody.join(",") === "2,3,4,5", "video range body is invalid");

const openRange = await fetchWorker("/__airwiki-protected/assets/airwiki-demo.mp4", {
  headers: { Range: "bytes=7-" },
});
assert(openRange.status === 206, `open video range returned ${openRange.status}`);
assert(openRange.headers.get("content-range") === "bytes 7-9/10", "open video range is invalid");

const suffixRange = await fetchWorker("/__airwiki-protected/assets/airwiki-demo.mp4", {
  headers: { Range: "bytes=-3" },
});
assert(suffixRange.status === 206, `suffix video range returned ${suffixRange.status}`);
assert(
  suffixRange.headers.get("content-range") === "bytes 7-9/10",
  "suffix video range is invalid",
);

const unsatisfiableRange = await fetchWorker(
  "/__airwiki-protected/assets/airwiki-demo.mp4",
  { headers: { Range: "bytes=20-30" } },
);
assert(
  unsatisfiableRange.status === 416,
  `unsatisfiable video range returned ${unsatisfiableRange.status}`,
);
assert(
  unsatisfiableRange.headers.get("content-range") === "bytes */10",
  "unsatisfiable video range is invalid",
);
assertSecurityHeaders(unsatisfiableRange, "unsatisfiable video range");

const multipleRange = await fetchWorker("/__airwiki-protected/assets/airwiki-demo.mp4", {
  headers: { Range: "bytes=0-1,3-4" },
});
assert(multipleRange.status === 200, `multiple video range returned ${multipleRange.status}`);
assert(multipleRange.headers.get("content-range") === null, "multiple video range was synthesized");
assert((await multipleRange.arrayBuffer()).byteLength === 10, "multiple video range lost content");

const malformedRange = await fetchWorker("/__airwiki-protected/assets/airwiki-demo.mp4", {
  headers: { Range: "items=0-3" },
});
assert(malformedRange.status === 200, `malformed video range returned ${malformedRange.status}`);
assert(malformedRange.headers.get("content-range") === null, "malformed video range was synthesized");
assert((await malformedRange.arrayBuffer()).byteLength === 10, "malformed video range lost content");

const headRange = await fetchWorker("/__airwiki-protected/assets/airwiki-demo.mp4", {
  method: "HEAD",
  headers: { Range: "bytes=2-5" },
});
assert(headRange.status === 206, `HEAD video range returned ${headRange.status}`);
assert(headRange.headers.get("content-range") === "bytes 2-5/10", "HEAD video range is invalid");
assert((await headRange.text()) === "", "HEAD video range returned a body");
assertSecurityHeaders(headRange, "HEAD video range");

const headRangeWithoutLength = await worker.fetch(
  new Request("https://preview.invalid/__airwiki-protected/assets/airwiki-demo.mp4", {
    method: "HEAD",
    headers: { Range: "bytes=2-5" },
  }),
  {
    ASSETS: {
      async fetch() {
        return new Response(null, { headers: { "Content-Type": "video/mp4" } });
      },
    },
  },
  {},
);
assert(
  headRangeWithoutLength.status === 200,
  `lengthless HEAD range returned ${headRangeWithoutLength.status}`,
);
assert(
  headRangeWithoutLength.headers.get("content-range") === null,
  "lengthless HEAD range invented a Content-Range",
);
assertSecurityHeaders(headRangeWithoutLength, "lengthless HEAD range");

const nativeRange = await worker.fetch(
  new Request("https://preview.invalid/__airwiki-protected/assets/airwiki-demo.mp4", {
    headers: { Range: "bytes=2-5" },
  }),
  {
    ASSETS: {
      async fetch() {
        return new Response(new Uint8Array([2, 3, 4, 5]), {
          status: 206,
          headers: {
            "Content-Length": "4",
            "Content-Range": "bytes 2-5/10",
            "Content-Type": "video/mp4",
          },
        });
      },
    },
  },
  {},
);
assert(nativeRange.status === 206, `native asset range returned ${nativeRange.status}`);
assert(nativeRange.headers.get("content-range") === "bytes 2-5/10", "native asset range changed");
assertSecurityHeaders(nativeRange, "native asset range");
