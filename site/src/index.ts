import appScript from "../app.js?raw";
import htmlSource from "../index.html?raw";
import launchConfigScript from "../launch-config.js?raw";
import stylesheet from "../styles.css?raw";

const securityHeaders = {
  "Content-Security-Policy":
    "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; media-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'",
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Resource-Policy": "same-origin",
  "Permissions-Policy":
    "accelerometer=(), ambient-light-sensor=(), autoplay=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()",
  "Referrer-Policy": "no-referrer",
  "X-Content-Type-Options": "nosniff",
  "X-Frame-Options": "DENY",
} as const;

const protectedPrefix = "/__airwiki-protected";
const protectedAssetPrefix = `${protectedPrefix}/assets/`;
const assetNames = new Set([
  "airwiki-demo-poster.png",
  "airwiki-demo.mp4",
  "airwiki-mark.png",
  "airwiki-review-flow.png",
]);

const protectedHtml = htmlSource
  .replaceAll('="assets/', `="${protectedAssetPrefix}`)
  .replace('href="styles.css"', `href="${protectedPrefix}/styles.css"`)
  .replace('src="launch-config.js"', `src="${protectedPrefix}/launch-config.js"`)
  .replace('src="app.js"', `src="${protectedPrefix}/app.js"`);

const textRoutes = new Map<string, { body: string; contentType: string }>([
  ["/", { body: protectedHtml, contentType: "text/html; charset=utf-8" }],
  ["/index.html", { body: protectedHtml, contentType: "text/html; charset=utf-8" }],
  [
    `${protectedPrefix}/styles.css`,
    { body: stylesheet, contentType: "text/css; charset=utf-8" },
  ],
  [
    `${protectedPrefix}/launch-config.js`,
    { body: launchConfigScript, contentType: "text/javascript; charset=utf-8" },
  ],
  [
    `${protectedPrefix}/app.js`,
    { body: appScript, contentType: "text/javascript; charset=utf-8" },
  ],
]);

interface Env {
  ASSETS: Fetcher;
}

function applySecurityHeaders(headers: Headers): void {
  for (const [name, value] of Object.entries(securityHeaders)) {
    headers.set(name, value);
  }
}

function textResponse(
  request: Request,
  body: string,
  contentType: string,
  status = 200,
): Response {
  const headers = new Headers({ "Content-Type": contentType });
  applySecurityHeaders(headers);
  return new Response(request.method === "HEAD" ? null : body, { status, headers });
}

interface ByteRange {
  end: number;
  start: number;
}

type ByteRangeResult = ByteRange | "ignore" | "unsatisfiable";

function isSingleByteRange(value: string): boolean {
  const match = /^bytes=(\d*)-(\d*)$/.exec(value);
  return Boolean(match && (match[1] !== "" || match[2] !== ""));
}

function parseByteRange(value: string, size: number): ByteRangeResult {
  const match = /^bytes=(\d*)-(\d*)$/.exec(value);
  if (!match) return "ignore";

  const [, startText, endText] = match;
  if (startText === "" && endText === "") return "ignore";
  if (size === 0) return "unsatisfiable";

  if (startText === "") {
    const suffixLength = Number(endText);
    if (!Number.isSafeInteger(suffixLength) || suffixLength <= 0) {
      return "unsatisfiable";
    }
    return { start: Math.max(size - suffixLength, 0), end: size - 1 };
  }

  const start = Number(startText);
  const requestedEnd = endText === "" ? size - 1 : Number(endText);
  if (
    !Number.isSafeInteger(start) ||
    !Number.isSafeInteger(requestedEnd) ||
    start < 0 ||
    requestedEnd < start ||
    start >= size
  ) {
    return "unsatisfiable";
  }

  return { start, end: Math.min(requestedEnd, size - 1) };
}

async function securedAssetResponse(
  request: Request,
  response: Response,
  assetName: string,
): Promise<Response> {
  const headers = new Headers(response.headers);
  applySecurityHeaders(headers);

  if (assetName === "airwiki-demo.mp4") {
    headers.set("Accept-Ranges", "bytes");
    const rangeHeader = request.headers.get("Range");
    if (rangeHeader && response.status === 200) {
      const contentLengthHeader = headers.get("Content-Length");
      const contentLength = contentLengthHeader === null ? null : Number(contentLengthHeader);
      if (
        request.method === "HEAD" &&
        (contentLength === null || !Number.isSafeInteger(contentLength) || contentLength < 0)
      ) {
        return new Response(null, { status: response.status, headers });
      }
      const bytes = request.method === "HEAD" ? null : await response.arrayBuffer();
      const size = bytes?.byteLength ?? contentLength;
      const range = size !== null && Number.isSafeInteger(size) && size >= 0
        ? parseByteRange(rangeHeader, size)
        : "ignore";

      if (range === "ignore") {
        if (bytes) headers.set("Content-Length", String(bytes.byteLength));
        return new Response(bytes, { status: response.status, headers });
      }

      if (range === "unsatisfiable") {
        headers.set(
          "Content-Range",
          `bytes */${size !== null && Number.isSafeInteger(size) ? size : "*"}`,
        );
        headers.set("Content-Length", "0");
        return new Response(null, { status: 416, headers });
      }

      const length = range.end - range.start + 1;
      headers.set("Content-Range", `bytes ${range.start}-${range.end}/${size}`);
      headers.set("Content-Length", String(length));
      return new Response(bytes?.slice(range.start, range.end + 1) ?? null, {
        status: 206,
        headers,
      });
    }
  }

  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

export default {
  async fetch(request, env) {
    if (request.method !== "GET" && request.method !== "HEAD") {
      const response = textResponse(
        request,
        "Method not allowed\n",
        "text/plain; charset=utf-8",
        405,
      );
      response.headers.set("Allow", "GET, HEAD");
      return response;
    }

    const url = new URL(request.url);
    const textRoute = textRoutes.get(url.pathname);
    if (textRoute) {
      return textResponse(request, textRoute.body, textRoute.contentType);
    }

    if (url.pathname.startsWith(protectedAssetPrefix)) {
      const assetName = url.pathname.slice(protectedAssetPrefix.length);
      if (!assetNames.has(assetName)) {
        return textResponse(request, "Not found\n", "text/plain; charset=utf-8", 404);
      }

      const assetUrl = new URL(`/assets/${assetName}`, request.url);
      let assetRequest = new Request(assetUrl, request);
      const rangeHeader = request.headers.get("Range");
      if (assetName === "airwiki-demo.mp4" && rangeHeader && !isSingleByteRange(rangeHeader)) {
        const headers = new Headers(assetRequest.headers);
        headers.delete("Range");
        assetRequest = new Request(assetRequest, { headers });
      }
      const assetResponse = await env.ASSETS.fetch(assetRequest);
      return securedAssetResponse(request, assetResponse, assetName);
    }

    return textResponse(request, "Not found\n", "text/plain; charset=utf-8", 404);
  },
} satisfies ExportedHandler<Env>;
