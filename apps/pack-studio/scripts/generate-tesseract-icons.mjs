#!/usr/bin/env node
/**
 * Generate minimalist monochrome Voxel Tesseract app icons & README SVGs
 * using @resvg/resvg-js (Vercel / Zed / Linear aesthetic).
 *
 * Outputs:
 *   apps/pack-studio/src-tauri/icons/:
 *     - icon.png        (1024x1024 master PNG)
 *     - 32x32.png       (32x32 PNG)
 *     - 128x128.png     (128x128 PNG)
 *     - 128x128@2x.png  (256x256 PNG)
 *     - icon.ico        (Windows multi-size ICO: 16, 32, 48, 64, 128, 256)
 *     - icon.icns       (macOS multi-size ICNS: 16, 32, 128, 256, 512, 1024)
 *   docs/assets/:
 *     - packstudio-logo-dark.svg   (pure white #FFFFFF on transparent bg)
 *     - packstudio-logo-light.svg  (pure obsidian #09090B on transparent bg)
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Resvg } from "@resvg/resvg-js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const iconsDir = path.resolve(__dirname, "../src-tauri/icons");
const docsAssetsDir = path.resolve(__dirname, "../../../docs/assets");
fs.mkdirSync(iconsDir, { recursive: true });
fs.mkdirSync(docsAssetsDir, { recursive: true });

/**
 * Pure monochrome isometric Voxel Tesseract geometry (1024x1024 coordinate space).
 * Zero gradients, zero blur/glow — relies purely on crisp line weights and flat
 * opacity steps (1.0 / 0.72 / 0.45) for the inner isometric voxel core.
 */
function buildTesseractGeometry(ink) {
  return `
  <!-- Subtle back edges of outer wireframe cube -->
  <g stroke="${ink}" stroke-opacity="0.24" stroke-width="10" stroke-linecap="round" fill="none">
    <line x1="512" y1="168" x2="512" y2="548"/>
    <line x1="512" y1="548" x2="224" y2="698"/>
    <line x1="512" y1="548" x2="800" y2="698"/>
  </g>

  <!-- 8 Crisp Hyper-edges connecting outer cube vertices to inner voxel core -->
  <g stroke="${ink}" stroke-width="10" stroke-linecap="round" fill="none">
    <line x1="512" y1="548" x2="512" y2="528" stroke-opacity="0.28"/>
    <line x1="512" y1="168" x2="512" y2="354" stroke-opacity="0.62"/>
    <line x1="224" y1="326" x2="380" y2="426" stroke-opacity="0.62"/>
    <line x1="800" y1="326" x2="644" y2="426" stroke-opacity="0.62"/>
    <line x1="224" y1="698" x2="380" y2="598" stroke-opacity="0.62"/>
    <line x1="800" y1="698" x2="644" y2="598" stroke-opacity="0.62"/>
    <line x1="512" y1="856" x2="512" y2="670" stroke-opacity="0.62"/>
    <line x1="512" y1="476" x2="512" y2="496" stroke-opacity="0.78"/>
  </g>

  <!-- Inner solid faceted Voxel Core (flat opacity steps: 1.0 / 0.72 / 0.45) -->
  <g>
    <polygon points="512,354 644,426 512,496 380,426" fill="${ink}" fill-opacity="1.0"/>
    <polygon points="380,426 512,496 512,670 380,598" fill="${ink}" fill-opacity="0.72"/>
    <polygon points="644,426 512,496 512,670 644,598" fill="${ink}" fill-opacity="0.45"/>
    <g stroke="${ink}" stroke-opacity="0.92" stroke-width="6" stroke-linejoin="round" stroke-linecap="round" fill="none">
      <polygon points="512,354 644,426 644,598 512,670 380,598 380,426"/>
      <line x1="380" y1="426" x2="512" y2="496"/>
      <line x1="644" y1="426" x2="512" y2="496"/>
      <line x1="512" y1="496" x2="512" y2="670"/>
    </g>
  </g>

  <!-- Outer wireframe cube (crisp solid monochrome edges) -->
  <g stroke="${ink}" stroke-width="18" stroke-linejoin="round" stroke-linecap="round" fill="none">
    <polygon points="512,168 800,326 800,698 512,856 224,698 224,326"/>
    <line x1="224" y1="326" x2="512" y2="476"/>
    <line x1="800" y1="326" x2="512" y2="476"/>
    <line x1="512" y1="476" x2="512" y2="856"/>
  </g>`;
}

// Master desktop app icon: matte obsidian tile (#09090B) + subtle #27272A inner border + pure #FFFFFF geometry
const APP_ICON_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <rect x="44" y="44" width="936" height="936" rx="212" fill="#09090B"/>
  <rect x="52" y="52" width="920" height="920" rx="204" fill="none" stroke="#27272A" stroke-width="10"/>
${buildTesseractGeometry("#FFFFFF")}
</svg>
`;

// Transparent-background README SVGs (tightly framed viewBox for clean floating placement)
function buildReadmeSvg(ink) {
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="160 120 704 784" width="256" height="285">
${buildTesseractGeometry(ink)}
</svg>
`;
}

function renderPng(size) {
  const resvg = new Resvg(APP_ICON_SVG, {
    fitTo: {
      mode: "width",
      value: size,
    },
  });
  const rendered = resvg.render();
  return Buffer.from(rendered.asPng());
}

/**
 * Build a Windows multi-resolution .ico containing PNG-encoded entries.
 */
function buildIco(entries) {
  const count = entries.length;
  const headerSize = 6 + count * 16;
  const header = Buffer.alloc(headerSize);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(count, 4);

  let offset = headerSize;
  const buffers = [header];

  entries.forEach(({ size, data }, i) => {
    const pos = 6 + i * 16;
    header.writeUInt8(size >= 256 ? 0 : size, pos);
    header.writeUInt8(size >= 256 ? 0 : size, pos + 1);
    header.writeUInt8(0, pos + 2);
    header.writeUInt8(0, pos + 3);
    header.writeUInt16LE(1, pos + 4);
    header.writeUInt16LE(32, pos + 6);
    header.writeUInt32LE(data.length, pos + 8);
    header.writeUInt32LE(offset, pos + 12);
    offset += data.length;
    buffers.push(data);
  });

  return Buffer.concat(buffers);
}

/**
 * Build a standard macOS .icns container using PNG-encoded OSType chunks.
 */
function buildIcns(entries) {
  const chunks = [];
  let totalLength = 8;

  for (const { osType, data } of entries) {
    const chunkHeader = Buffer.alloc(8);
    chunkHeader.write(osType, 0, 4, "ascii");
    chunkHeader.writeUInt32BE(8 + data.length, 4);
    chunks.push(chunkHeader, data);
    totalLength += 8 + data.length;
  }

  const fileHeader = Buffer.alloc(8);
  fileHeader.write("icns", 0, 4, "ascii");
  fileHeader.writeUInt32BE(totalLength, 4);

  return Buffer.concat([fileHeader, ...chunks]);
}

// Export transparent-background README SVGs
fs.writeFileSync(
  path.join(docsAssetsDir, "packstudio-logo-dark.svg"),
  buildReadmeSvg("#FFFFFF"),
);
fs.writeFileSync(
  path.join(docsAssetsDir, "packstudio-logo-light.svg"),
  buildReadmeSvg("#09090B"),
);

// Render all PNG sizes from master matte obsidian + pure white SVG
const sizesNeeded = [16, 32, 48, 64, 128, 256, 512, 1024];
const pngBySize = new Map();
for (const size of sizesNeeded) {
  pngBySize.set(size, renderPng(size));
}

fs.writeFileSync(path.join(iconsDir, "icon.png"), pngBySize.get(1024));
fs.writeFileSync(path.join(iconsDir, "32x32.png"), pngBySize.get(32));
fs.writeFileSync(path.join(iconsDir, "128x128.png"), pngBySize.get(128));
fs.writeFileSync(path.join(iconsDir, "128x128@2x.png"), pngBySize.get(256));

const icoBuffer = buildIco([
  { size: 16, data: pngBySize.get(16) },
  { size: 32, data: pngBySize.get(32) },
  { size: 48, data: pngBySize.get(48) },
  { size: 64, data: pngBySize.get(64) },
  { size: 128, data: pngBySize.get(128) },
  { size: 256, data: pngBySize.get(256) },
]);
fs.writeFileSync(path.join(iconsDir, "icon.ico"), icoBuffer);

const icnsBuffer = buildIcns([
  { osType: "icp4", data: pngBySize.get(16) },
  { osType: "icp5", data: pngBySize.get(32) },
  { osType: "ic07", data: pngBySize.get(128) },
  { osType: "ic08", data: pngBySize.get(256) },
  { osType: "ic09", data: pngBySize.get(512) },
  { osType: "ic10", data: pngBySize.get(1024) },
]);
fs.writeFileSync(path.join(iconsDir, "icon.icns"), icnsBuffer);

const outputs = [
  "icon.png",
  "32x32.png",
  "128x128.png",
  "128x128@2x.png",
  "icon.ico",
  "icon.icns",
];

console.log(`Generated monochrome PackStudio Voxel Tesseract icons in ${iconsDir}:`);
for (const file of outputs) {
  const stats = fs.statSync(path.join(iconsDir, file));
  console.log(`  - ${file.padEnd(15)} (${stats.size.toLocaleString()} bytes)`);
}
console.log(`Exported README SVGs in ${docsAssetsDir}:`);
for (const file of ["packstudio-logo-dark.svg", "packstudio-logo-light.svg"]) {
  const stats = fs.statSync(path.join(docsAssetsDir, file));
  console.log(`  - ${file.padEnd(26)} (${stats.size.toLocaleString()} bytes)`);
}
