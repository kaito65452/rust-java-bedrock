#!/usr/bin/env node
/**
 * Generate PackStudio Voxel Tesseract app icons using @resvg/resvg-js.
 *
 * Outputs into apps/pack-studio/src-tauri/icons/:
 *   - icon.png        (1024x1024 master PNG)
 *   - 32x32.png       (32x32 PNG)
 *   - 128x128.png     (128x128 PNG)
 *   - 128x128@2x.png  (256x256 PNG)
 *   - icon.ico        (Windows multi-size ICO: 16, 32, 48, 64, 128, 256)
 *   - icon.icns       (macOS multi-size ICNS: 16, 32, 128, 256, 512, 1024)
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Resvg } from "@resvg/resvg-js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const iconsDir = path.resolve(__dirname, "../src-tauri/icons");
fs.mkdirSync(iconsDir, { recursive: true });

// Master isometric Voxel Tesseract SVG (1024x1024 viewBox):
// Translucent outer wireframe cube connected via 8 hyper-edges to a glowing inner voxel core.
const TESSERACT_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <defs>
    <linearGradient id="bgGrad" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#0b1020"/>
      <stop offset="55%" stop-color="#131c33"/>
      <stop offset="100%" stop-color="#181736"/>
    </linearGradient>
    <radialGradient id="coreHalo" cx="50%" cy="50%" r="44%">
      <stop offset="0%" stop-color="#38bdf8" stop-opacity="0.42"/>
      <stop offset="50%" stop-color="#6366f1" stop-opacity="0.18"/>
      <stop offset="100%" stop-color="#0b1020" stop-opacity="0"/>
    </radialGradient>
    <linearGradient id="outerFaceTop" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#60a5fa" stop-opacity="0.22"/>
      <stop offset="100%" stop-color="#38bdf8" stop-opacity="0.06"/>
    </linearGradient>
    <linearGradient id="outerFaceLeft" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#3b82f6" stop-opacity="0.16"/>
      <stop offset="100%" stop-color="#6366f1" stop-opacity="0.05"/>
    </linearGradient>
    <linearGradient id="outerFaceRight" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="#8b5cf6" stop-opacity="0.18"/>
      <stop offset="100%" stop-color="#38bdf8" stop-opacity="0.06"/>
    </linearGradient>
    <linearGradient id="coreTop" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#6ee7b7"/>
      <stop offset="100%" stop-color="#38bdf8"/>
    </linearGradient>
    <linearGradient id="coreLeft" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="#22d3ee"/>
      <stop offset="100%" stop-color="#2563eb"/>
    </linearGradient>
    <linearGradient id="coreRight" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#818cf8"/>
      <stop offset="100%" stop-color="#4f46e5"/>
    </linearGradient>
    <linearGradient id="outerStroke" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#7dd3fc"/>
      <stop offset="50%" stop-color="#38bdf8"/>
      <stop offset="100%" stop-color="#a78bfa"/>
    </linearGradient>
    <linearGradient id="hyperEdge" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#c4b5fd" stop-opacity="0.95"/>
      <stop offset="100%" stop-color="#38bdf8" stop-opacity="0.75"/>
    </linearGradient>
  </defs>

  <!-- Rounded macOS/Linux/Windows obsidian app tile -->
  <rect x="44" y="44" width="936" height="936" rx="210" fill="url(#bgGrad)" stroke="#38bdf8" stroke-opacity="0.32" stroke-width="10"/>
  <rect x="66" y="66" width="892" height="892" rx="190" fill="none" stroke="#ffffff" stroke-opacity="0.06" stroke-width="4"/>

  <!-- Subtle radial ambient glow -->
  <circle cx="512" cy="512" r="390" fill="url(#coreHalo)"/>

  <!--
    Outer Cube Isometric Vertices (center 512, 512, radius ~336):
      T  = (512, 172)   top-back
      TL = (220, 342)   top-left
      TR = (804, 342)   top-right
      TC = (512, 512)   top-front (front-center upper)
      BL = (220, 682)   bottom-left
      BR = (804, 682)   bottom-right
      BC = (512, 852)   bottom-front
      BK = (512, 512) -> for true 3D perspective tilt, shift back-bottom slightly up:
        Let's use a genuine 3D orthographic-perspective projection of unit cube [-1, 1]^3
        rotated yaw = 38 deg, pitch = 26 deg so all 8 outer + 8 inner vertices are distinct!

    Projected coordinates (precomputed for crisp symmetry):
      Outer cube (scale = 235):
        O0 (-1,-1,-1) back-bottom:   (512, 650)
        O1 (+1,-1,-1) right-back-b:  (770, 560)
        O2 (+1,+1,-1) right-back-t:  (770, 290)
        O3 (-1,+1,-1) back-top:      (512, 180)
        O4 (-1,-1,+1) left-front-b:  (254, 734)
        O5 (+1,-1,+1) front-bottom:  (512, 844)
        O6 (+1,+1,+1) front-top:     (512, 574) -> wait: let's use exact symmetric isometric + subtle perspective!

    Exact isometric-perspective coordinates (symmetric around x=512):
      Outer cube (8 distinct vertices):
        OTB (top-back):     512, 168
        OTL (top-left):     224, 326
        OTR (top-right):    800, 326
        OTF (top-front):    512, 476
        OBB (bottom-back):  512, 548
        OBL (bottom-left):  224, 698
        OBR (bottom-right): 800, 698
        OBF (bottom-front): 512, 856

      Inner voxel core (0.46x scaled around center 512, 512):
        ITB (top-back):     512, 354
        ITL (top-left):     380, 426
        ITR (top-right):    644, 426
        ITF (top-front):    512, 496
        IBB (bottom-back):  512, 528
        IBL (bottom-left):  380, 598
        IBR (bottom-right): 644, 598
        IBF (bottom-front): 512, 670
  -->

  <!-- Back wireframe edges of outer cube (dashed/faint depth cue) -->
  <g stroke="#38bdf8" stroke-opacity="0.32" stroke-width="11" stroke-linecap="round" fill="none">
    <line x1="512" y1="168" x2="512" y2="548"/>
    <line x1="512" y1="548" x2="224" y2="698"/>
    <line x1="512" y1="548" x2="800" y2="698"/>
  </g>

  <!-- Translucent outer cube glass faces -->
  <polygon points="512,168 800,326 512,476 224,326" fill="url(#outerFaceTop)"/>
  <polygon points="224,326 512,476 512,856 224,698" fill="url(#outerFaceLeft)"/>
  <polygon points="800,326 512,476 512,856 800,698" fill="url(#outerFaceRight)"/>

  <!-- 8 Hyper-edges connecting Outer Cube vertices to Inner Voxel Core vertices -->
  <g stroke="url(#hyperEdge)" stroke-width="11" stroke-linecap="round" fill="none">
    <!-- Back hyper-edges (slightly softer) -->
    <line x1="512" y1="548" x2="512" y2="528" stroke-opacity="0.45"/>
    <!-- 7 visible hyper-edges -->
    <line x1="512" y1="168" x2="512" y2="354" stroke-opacity="0.88"/>
    <line x1="224" y1="326" x2="380" y2="426" stroke-opacity="0.92"/>
    <line x1="800" y1="326" x2="644" y2="426" stroke-opacity="0.92"/>
    <line x1="224" y1="698" x2="380" y2="598" stroke-opacity="0.92"/>
    <line x1="800" y1="698" x2="644" y2="598" stroke-opacity="0.92"/>
    <line x1="512" y1="856" x2="512" y2="670" stroke-opacity="0.95"/>
    <line x1="512" y1="476" x2="512" y2="496" stroke-opacity="0.95"/>
  </g>

  <!-- Inner glowing Voxel Core (solid beveled faces + subtle voxel grid seams) -->
  <g>
    <polygon points="512,354 644,426 512,496 380,426" fill="url(#coreTop)"/>
    <polygon points="380,426 512,496 512,670 380,598" fill="url(#coreLeft)"/>
    <polygon points="644,426 512,496 512,670 644,598" fill="url(#coreRight)"/>

    <!-- Subtle 2x2 voxel facet cross-lines on the inner core -->
    <g stroke="#ffffff" stroke-opacity="0.24" stroke-width="4" fill="none">
      <line x1="446" y1="390" x2="578" y2="461"/>
      <line x1="578" y1="390" x2="446" y2="461"/>
      <line x1="380" y1="512" x2="512" y2="583"/>
      <line x1="446" y1="461" x2="446" y2="634"/>
      <line x1="644" y1="512" x2="512" y2="583"/>
      <line x1="578" y1="461" x2="578" y2="634"/>
    </g>

    <!-- Crisp glowing outline of the inner core -->
    <g stroke="#e0f2fe" stroke-opacity="0.9" stroke-width="9" stroke-linejoin="round" stroke-linecap="round" fill="none">
      <polygon points="512,354 644,426 644,598 512,670 380,598 380,426"/>
      <line x1="380" y1="426" x2="512" y2="496"/>
      <line x1="644" y1="426" x2="512" y2="496"/>
      <line x1="512" y1="496" x2="512" y2="670"/>
    </g>
  </g>

  <!-- Outer Cube front wireframe edges -->
  <g stroke="url(#outerStroke)" stroke-width="18" stroke-linejoin="round" stroke-linecap="round" fill="none">
    <polygon points="512,168 800,326 800,698 512,856 224,698 224,326"/>
    <line x1="224" y1="326" x2="512" y2="476"/>
    <line x1="800" y1="326" x2="512" y2="476"/>
    <line x1="512" y1="476" x2="512" y2="856"/>
  </g>

  <!-- Glowing vertex nodes on the outer cube -->
  <g fill="#e0f2fe" stroke="#38bdf8" stroke-width="6">
    <circle cx="512" cy="168" r="14"/>
    <circle cx="224" cy="326" r="14"/>
    <circle cx="800" cy="326" r="14"/>
    <circle cx="512" cy="476" r="15"/>
    <circle cx="224" cy="698" r="14"/>
    <circle cx="800" cy="698" r="14"/>
    <circle cx="512" cy="856" r="14"/>
  </g>
</svg>
`;

function renderPng(size) {
  const resvg = new Resvg(TESSERACT_SVG, {
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
 * Vista+ / Windows 10 / 11 and Tauri NSIS/MSI natively support PNG-in-ICO.
 */
function buildIco(entries) {
  const count = entries.length;
  const headerSize = 6 + count * 16;
  const header = Buffer.alloc(headerSize);
  header.writeUInt16LE(0, 0); // reserved
  header.writeUInt16LE(1, 2); // type = ICO
  header.writeUInt16LE(count, 4);

  let offset = headerSize;
  const buffers = [header];

  entries.forEach(({ size, data }, i) => {
    const pos = 6 + i * 16;
    header.writeUInt8(size >= 256 ? 0 : size, pos); // width (0 = 256)
    header.writeUInt8(size >= 256 ? 0 : size, pos + 1); // height (0 = 256)
    header.writeUInt8(0, pos + 2); // color palette
    header.writeUInt8(0, pos + 3); // reserved
    header.writeUInt16LE(1, pos + 4); // color planes
    header.writeUInt16LE(32, pos + 6); // bits per pixel
    header.writeUInt32LE(data.length, pos + 8); // image bytes length
    header.writeUInt32LE(offset, pos + 12); // offset
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

const sizesNeeded = [16, 32, 48, 64, 128, 256, 512, 1024];
const pngBySize = new Map();
for (const size of sizesNeeded) {
  pngBySize.set(size, renderPng(size));
}

// Write Tauri PNG icons
fs.writeFileSync(path.join(iconsDir, "icon.png"), pngBySize.get(1024));
fs.writeFileSync(path.join(iconsDir, "32x32.png"), pngBySize.get(32));
fs.writeFileSync(path.join(iconsDir, "128x128.png"), pngBySize.get(128));
fs.writeFileSync(path.join(iconsDir, "128x128@2x.png"), pngBySize.get(256));

// Write Windows .ico
const icoBuffer = buildIco([
  { size: 16, data: pngBySize.get(16) },
  { size: 32, data: pngBySize.get(32) },
  { size: 48, data: pngBySize.get(48) },
  { size: 64, data: pngBySize.get(64) },
  { size: 128, data: pngBySize.get(128) },
  { size: 256, data: pngBySize.get(256) },
]);
fs.writeFileSync(path.join(iconsDir, "icon.ico"), icoBuffer);

// Write macOS .icns
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

console.log(`Generated PackStudio Voxel Tesseract icons in ${iconsDir}:`);
for (const file of outputs) {
  const stats = fs.statSync(path.join(iconsDir, file));
  console.log(`  - ${file.padEnd(15)} (${stats.size.toLocaleString()} bytes)`);
}
