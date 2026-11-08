import { afterAll, expect, test } from "bun:test";
import { mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  imageType,
  isPrivateAddress,
  manifestIcons,
  parseSizes,
  rankCandidates,
  refreshLogos,
  scanPage,
} from "../src/logos";

const dir = mkdtempSync("/tmp/fn-admin-logos-");
afterAll(() => rmSync(dir, { recursive: true, force: true }));

const PAGE = `<!doctype html><html><head>
  <link rel="icon" href="/favicon-32.png" sizes="32x32">
  <link rel="icon" href="/logo.svg" type="image/svg+xml">
  <link rel="apple-touch-icon" href="/apple.png">
  <link rel="Shortcut Icon" href="https://cdn.example.org/big.png" sizes="16x16 256x256">
  <link rel="manifest" href="/site.webmanifest">
  <meta property="og:image" content="//cdn.example.org/share.jpg">
  <link rel="icon" href="javascript:alert(1)">
</head><body></body></html>`;

test("finds icons, the preview image and the manifest in a page", async () => {
  const { candidates, manifest } = await scanPage(PAGE, "https://www.example.org/start/");
  expect(manifest).toBe("https://www.example.org/site.webmanifest");
  expect(candidates).toEqual([
    { url: "https://www.example.org/favicon-32.png", size: 32, kind: "icon" },
    { url: "https://www.example.org/apple.png", size: 180, kind: "icon" },
    { url: "https://cdn.example.org/big.png", size: 256, kind: "icon" },
    { url: "https://cdn.example.org/share.jpg", size: 0, kind: "preview" },
    { url: "https://www.example.org/favicon.ico", size: 0, kind: "favicon" },
  ]);
});

test("ranks large icons first, then the preview, small icons, favicon; no SVG", () => {
  const ranked = rankCandidates([
    { url: "https://a.org/favicon.ico", size: 0, kind: "favicon" },
    { url: "https://a.org/16.png", size: 16, kind: "icon" },
    { url: "https://a.org/share.jpg", size: 0, kind: "preview" },
    { url: "https://a.org/192.png", size: 192, kind: "icon" },
    { url: "https://a.org/512.png", size: 512, kind: "icon" },
    { url: "https://a.org/logo.svg", size: 512, kind: "icon" },
    { url: "https://a.org/192.png", size: 0, kind: "favicon" }, // duplicate: best rank kept
  ]);
  expect(ranked.map((c) => c.url)).toEqual([
    "https://a.org/512.png",
    "https://a.org/192.png",
    "https://a.org/share.jpg",
    "https://a.org/16.png",
    "https://a.org/favicon.ico",
  ]);
});

test("manifest icons: sizes, relative URLs, no SVG or monochrome", () => {
  const icons = manifestIcons(
    {
      icons: [
        { src: "icons/192.png", sizes: "192x192", type: "image/png" },
        { src: "icons/any.svg", sizes: "any", type: "image/svg+xml" },
        { src: "icons/mono.png", sizes: "512x512", purpose: "monochrome" },
        { src: "icons/mask.png", sizes: "512x512", purpose: "maskable any" },
        { sizes: "64x64" },
      ],
    },
    "https://example.org/app/manifest.json",
  );
  expect(icons).toEqual([
    { url: "https://example.org/app/icons/192.png", size: 192, kind: "icon" },
    { url: "https://example.org/app/icons/mask.png", size: 512, kind: "icon" },
  ]);
  expect(manifestIcons("nope", "https://example.org/")).toEqual([]);
  expect(parseSizes("16x16 48X48 any")).toBe(48);
  expect(parseSizes(null)).toBe(0);
});

test("only real pictures count (magic bytes)", () => {
  expect(imageType(new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0]))).toBe("image/png");
  expect(imageType(new Uint8Array([0xff, 0xd8, 0xff, 0xe0]))).toBe("image/jpeg");
  expect(imageType(new TextEncoder().encode("RIFF....WEBPVP8 "))).toBe("image/webp");
  expect(imageType(new Uint8Array([0, 0, 1, 0, 1]))).toBe("image/x-icon");
  expect(imageType(new TextEncoder().encode("<svg xmlns="))).toBeNull();
  expect(imageType(new TextEncoder().encode("<!doctype html>"))).toBeNull();
});

test("home-network addresses are refused", () => {
  for (const ip of ["127.0.0.1", "10.1.2.3", "192.168.1.1", "172.16.0.1", "172.31.255.255", "169.254.1.1",
    "100.64.0.1", "0.0.0.0", "::1", "fe80::1", "fd00::1", "::ffff:192.168.1.1", "224.0.0.1"]) {
    expect(isPrivateAddress(ip)).toBe(true);
  }
  for (const ip of ["93.184.216.34", "172.32.0.1", "8.8.8.8", "2a00:1450:4001::1", "::ffff:8.8.8.8"]) {
    expect(isPrivateAddress(ip)).toBe(false);
  }
});

test("names resolving into the home network are not fetched; turned-off logos are removed", async () => {
  writeFileSync(join(dir, "old.img"), "x");
  writeFileSync(join(dir, "old.json"), "{}");
  writeFileSync(join(dir, "notes.txt"), "not ours"); // other files stay
  const site = { id: "evil", name: "Evil", url: "https://rebind.example.org/", allow_subdomains: true, show_logo: true };
  const changed = await refreshLogos(dir, [site], { resolveHost: async () => ["192.168.1.10"] });
  expect(changed).toBe(true); // old.* removed
  expect(readdirSync(dir).sort()).toEqual(["notes.txt"]); // nothing fetched for "evil"
});
