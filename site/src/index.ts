import appScript from "../app.js?raw";
import htmlSource from "../index.html?raw";
import launchConfigScript from "../launch-config.js?raw";
import stylesheet from "../styles.css?raw";
import demoPosterDataUrl from "./assets/airwiki-demo-poster.png?inline";
import demoVideoDataUrl from "./assets/airwiki-demo.mp4?inline";
import markDataUrl from "./assets/airwiki-mark.png?inline";
import reviewFlowDataUrl from "./assets/airwiki-review-flow.png?inline";

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

interface ProtectedAsset {
  bytes?: Uint8Array<ArrayBuffer>;
  contentType: string;
  dataUrl: string;
  size: number;
}

function decodeDataUrl(dataUrl: string): Uint8Array<ArrayBuffer> {
  const separator = dataUrl.indexOf(",");
  if (separator === -1 || !dataUrl.slice(0, separator).endsWith(";base64")) {
    throw new Error("Bundled launch asset is not a base64 data URL");
  }

  const encoded = dataUrl.slice(separator + 1);
  const binary = atob(encoded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

const protectedAssets = new Map<string, ProtectedAsset>([
  [
    "airwiki-demo-poster.png",
    { contentType: "image/png", dataUrl: demoPosterDataUrl, size: 106_092 },
  ],
  [
    "airwiki-demo.mp4",
    { contentType: "video/mp4", dataUrl: demoVideoDataUrl, size: 663_509 },
  ],
  [
    "airwiki-mark.png",
    { contentType: "image/png", dataUrl: markDataUrl, size: 877_237 },
  ],
  [
    "airwiki-review-flow.png",
    { contentType: "image/png", dataUrl: reviewFlowDataUrl, size: 191_525 },
  ],
]);

function assetBytes(asset: ProtectedAsset): Uint8Array<ArrayBuffer> {
  asset.bytes ??= decodeDataUrl(asset.dataUrl);
  return asset.bytes;
}

const protectedHtml = htmlSource.replaceAll(" vite-ignore", "");

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

function securedAssetResponse(
  request: Request,
  asset: ProtectedAsset,
  assetName: string,
): Response {
  const bytes = request.method === "HEAD" ? undefined : assetBytes(asset);
  const headers = new Headers({
    "Content-Length": String(asset.size),
    "Content-Type": asset.contentType,
  });
  applySecurityHeaders(headers);

  if (assetName === "airwiki-demo.mp4") {
    headers.set("Accept-Ranges", "bytes");
    const rangeHeader = request.headers.get("Range");
    if (rangeHeader && isSingleByteRange(rangeHeader)) {
      const range = parseByteRange(rangeHeader, asset.size);

      if (range === "ignore") {
        return new Response(bytes ?? null, {
          status: 200,
          headers,
        });
      }

      if (range === "unsatisfiable") {
        headers.set("Content-Range", `bytes */${asset.size}`);
        headers.set("Content-Length", "0");
        return new Response(null, { status: 416, headers });
      }

      const length = range.end - range.start + 1;
      headers.set(
        "Content-Range",
        `bytes ${range.start}-${range.end}/${asset.size}`,
      );
      headers.set("Content-Length", String(length));
      return new Response(
        bytes?.slice(range.start, range.end + 1) ?? null,
        {
        status: 206,
        headers,
        },
      );
    }
  }

  return new Response(bytes ?? null, {
    status: 200,
    headers,
  });
}

export default {
  async fetch(request) {
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
      const asset = protectedAssets.get(assetName);
      if (!asset) {
        return textResponse(request, "Not found\n", "text/plain; charset=utf-8", 404);
      }
      return securedAssetResponse(request, asset, assetName);
    }

    return textResponse(request, "Not found\n", "text/plain; charset=utf-8", 404);
  },
} satisfies ExportedHandler;
