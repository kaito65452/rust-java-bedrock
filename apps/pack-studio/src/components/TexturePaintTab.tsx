import React, { useEffect, useRef, useState } from "react";
import type { BinaryAssetData } from "../types";

type PaintTool = "pan" | "brush" | "eraser" | "bucket" | "eyedropper";
type BiomeTintPreset = "none" | "plains_grass" | "oak_foliage" | "water";

const BIOME_TINTS: Record<BiomeTintPreset, [number, number, number] | null> = {
  none: null,
  plains_grass: [121, 192, 90],
  oak_foliage: [72, 181, 24],
  water: [63, 118, 228],
};

function hexToRgb(hex: string): [number, number, number] {
  const clean = hex.replace("#", "");
  const r = parseInt(clean.slice(0, 2) || "ff", 16);
  const g = parseInt(clean.slice(2, 4) || "ff", 16);
  const b = parseInt(clean.slice(4, 6) || "ff", 16);
  return [r, g, b];
}

function rgbToHex(r: number, g: number, b: number): string {
  const toHex = (v: number) => v.toString(16).padStart(2, "0");
  return `#${toHex(r)}${toHex(g)}${toHex(b)}`;
}

interface TexturePaintTabProps {
  relPath: string;
  asset: BinaryAssetData;
  onDirtyChange: (dirty: boolean) => void;
  onSaveBase64: (relPath: string, base64Png: string) => Promise<void>;
  onDownscale: (relPath: string, targetSize: number) => Promise<void>;
  onCompositeGrass: (relPath: string) => Promise<void>;
  onOpenBlockbench: (relPath: string) => void;
  onToast: (msg: string) => void;
}

export const TexturePaintTab: React.FC<TexturePaintTabProps> = ({
  relPath,
  asset,
  onDirtyChange,
  onSaveBase64,
  onDownscale,
  onCompositeGrass,
  onOpenBlockbench,
  onToast,
}) => {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const sourceImageDataRef = useRef<ImageData | null>(null);
  const undoStackRef = useRef<ImageData[]>([]);
  const redoStackRef = useRef<ImageData[]>([]);
  const paintingRef = useRef(false);
  const lastPixelRef = useRef<{ x: number; y: number } | null>(null);
  const panStartRef = useRef<{
    clientX: number;
    clientY: number;
    panX: number;
    panY: number;
  } | null>(null);

  const [imgWidth, setImgWidth] = useState<number>(asset.width ?? 16);
  const [imgHeight, setImgHeight] = useState<number>(asset.height ?? 16);
  const [tool, setTool] = useState<PaintTool>("brush");
  const [colorHex, setColorHex] = useState<string>("#55ff55");
  const [alphaVal, setAlphaVal] = useState<number>(255);
  const [zoom, setZoom] = useState<number>(16);
  const [pan, setPan] = useState<{ x: number; y: number }>({ x: 0, y: 0 });
  const [showGrid, setShowGrid] = useState<boolean>(true);
  const [alphaMask, setAlphaMask] = useState<boolean>(false);
  const [biomeTint, setBiomeTint] = useState<BiomeTintPreset>("none");
  const [saving, setSaving] = useState<boolean>(false);
  const [hoverPx, setHoverPx] = useState<{ x: number; y: number } | null>(null);
  const [canUndo, setCanUndo] = useState(false);
  const [canRedo, setCanRedo] = useState(false);

  const computeFitZoom = (w: number, h: number): number => {
    const vp = viewportRef.current;
    if (!vp) return w > 256 ? 0.5 : 12;
    const availW = Math.max(120, vp.clientWidth - 48);
    const availH = Math.max(120, vp.clientHeight - 48);
    const fit = Math.min(availW / Math.max(1, w), availH / Math.max(1, h));
    return Math.max(0.05, Math.min(48, Number(fit.toFixed(3))));
  };

  const renderToDisplayCanvas = () => {
    const canvas = canvasRef.current;
    const raw = sourceImageDataRef.current;
    if (!canvas || !raw) return;
    if (canvas.width !== raw.width || canvas.height !== raw.height) {
      canvas.width = raw.width;
      canvas.height = raw.height;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const tint = BIOME_TINTS[biomeTint];
    if (!alphaMask && !tint) {
      ctx.putImageData(raw, 0, 0);
      return;
    }

    const preview = new ImageData(
      new Uint8ClampedArray(raw.data),
      raw.width,
      raw.height,
    );
    const d = preview.data;
    for (let i = 0; i < d.length; i += 4) {
      if (alphaMask) {
        const a = d[i + 3];
        d[i] = a;
        d[i + 1] = a;
        d[i + 2] = a;
        d[i + 3] = 255;
      } else if (tint) {
        d[i] = Math.round((d[i] * tint[0]) / 255);
        d[i + 1] = Math.round((d[i + 1] * tint[1]) / 255);
        d[i + 2] = Math.round((d[i + 2] * tint[2]) / 255);
      }
    }
    ctx.putImageData(preview, 0, 0);
  };

  useEffect(() => {
    renderToDisplayCanvas();
  }, [alphaMask, biomeTint]);

  useEffect(() => {
    let cancelled = false;
    const img = new Image();
    img.onload = () => {
      if (cancelled) return;
      const w = img.naturalWidth || 16;
      const h = img.naturalHeight || 16;
      setImgWidth(w);
      setImgHeight(h);

      const off = document.createElement("canvas");
      off.width = w;
      off.height = h;
      const octx = off.getContext("2d");
      if (!octx) return;
      octx.clearRect(0, 0, w, h);
      octx.drawImage(img, 0, 0);
      sourceImageDataRef.current = octx.getImageData(0, 0, w, h);
      undoStackRef.current = [];
      redoStackRef.current = [];
      setCanUndo(false);
      setCanRedo(false);
      onDirtyChange(false);

      setZoom(computeFitZoom(w, h));
      setPan({ x: 0, y: 0 });
      renderToDisplayCanvas();
    };
    img.src = `data:image/png;base64,${asset.base64_png}`;
    return () => {
      cancelled = true;
    };
  }, [relPath, asset.base64_png]);

  const pushUndoSnapshot = () => {
    const cur = sourceImageDataRef.current;
    if (!cur) return;
    undoStackRef.current.push(
      new ImageData(new Uint8ClampedArray(cur.data), cur.width, cur.height),
    );
    if (undoStackRef.current.length > 40) {
      undoStackRef.current.shift();
    }
    redoStackRef.current = [];
    setCanUndo(undoStackRef.current.length > 0);
    setCanRedo(false);
  };

  const handleUndo = () => {
    const prev = undoStackRef.current.pop();
    const cur = sourceImageDataRef.current;
    if (!prev || !cur) return;
    redoStackRef.current.push(
      new ImageData(new Uint8ClampedArray(cur.data), cur.width, cur.height),
    );
    sourceImageDataRef.current = prev;
    setCanUndo(undoStackRef.current.length > 0);
    setCanRedo(redoStackRef.current.length > 0);
    renderToDisplayCanvas();
  };

  const handleRedo = () => {
    const next = redoStackRef.current.pop();
    const cur = sourceImageDataRef.current;
    if (!next || !cur) return;
    undoStackRef.current.push(
      new ImageData(new Uint8ClampedArray(cur.data), cur.width, cur.height),
    );
    sourceImageDataRef.current = next;
    setCanUndo(undoStackRef.current.length > 0);
    setCanRedo(redoStackRef.current.length > 0);
    renderToDisplayCanvas();
  };

  const getPixelCoords = (
    clientX: number,
    clientY: number,
  ): { x: number; y: number } | null => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    const px = Math.floor(((clientX - rect.left) / rect.width) * imgWidth);
    const py = Math.floor(((clientY - rect.top) / rect.height) * imgHeight);
    if (px < 0 || py < 0 || px >= imgWidth || py >= imgHeight) return null;
    return { x: px, y: py };
  };

  const setPixel = (
    data: ImageData,
    x: number,
    y: number,
    rgba: [number, number, number, number],
  ) => {
    if (x < 0 || y < 0 || x >= data.width || y >= data.height) return;
    const idx = (y * data.width + x) * 4;
    data.data[idx] = rgba[0];
    data.data[idx + 1] = rgba[1];
    data.data[idx + 2] = rgba[2];
    data.data[idx + 3] = rgba[3];
  };

  const drawBresenham = (
    x0: number,
    y0: number,
    x1: number,
    y1: number,
    rgba: [number, number, number, number],
  ) => {
    const cur = sourceImageDataRef.current;
    if (!cur) return;
    let cx = x0;
    let cy = y0;
    const dx = Math.abs(x1 - x0);
    const dy = -Math.abs(y1 - y0);
    const sx = x0 < x1 ? 1 : -1;
    const sy = y0 < y1 ? 1 : -1;
    let err = dx + dy;
    while (true) {
      setPixel(cur, cx, cy, rgba);
      if (cx === x1 && cy === y1) break;
      const e2 = 2 * err;
      if (e2 >= dy) {
        err += dy;
        cx += sx;
      }
      if (e2 <= dx) {
        err += dx;
        cy += sy;
      }
    }
    onDirtyChange(true);
    renderToDisplayCanvas();
  };

  const floodFill = (
    startX: number,
    startY: number,
    fillRgba: [number, number, number, number],
  ) => {
    const cur = sourceImageDataRef.current;
    if (!cur) return;
    const idx0 = (startY * cur.width + startX) * 4;
    const tr = cur.data[idx0];
    const tg = cur.data[idx0 + 1];
    const tb = cur.data[idx0 + 2];
    const ta = cur.data[idx0 + 3];
    if (
      tr === fillRgba[0] &&
      tg === fillRgba[1] &&
      tb === fillRgba[2] &&
      ta === fillRgba[3]
    ) {
      return;
    }
    pushUndoSnapshot();
    const stack: [number, number][] = [[startX, startY]];
    const visited = new Uint8Array(cur.width * cur.height);
    while (stack.length > 0) {
      const [x, y] = stack.pop()!;
      const pIdx = y * cur.width + x;
      if (visited[pIdx]) continue;
      const i4 = pIdx * 4;
      if (
        cur.data[i4] !== tr ||
        cur.data[i4 + 1] !== tg ||
        cur.data[i4 + 2] !== tb ||
        cur.data[i4 + 3] !== ta
      ) {
        continue;
      }
      visited[pIdx] = 1;
      cur.data[i4] = fillRgba[0];
      cur.data[i4 + 1] = fillRgba[1];
      cur.data[i4 + 2] = fillRgba[2];
      cur.data[i4 + 3] = fillRgba[3];
      if (x > 0) stack.push([x - 1, y]);
      if (x + 1 < cur.width) stack.push([x + 1, y]);
      if (y > 0) stack.push([x, y - 1]);
      if (y + 1 < cur.height) stack.push([x, y + 1]);
    }
    onDirtyChange(true);
    renderToDisplayCanvas();
  };

  const handleSave = async () => {
    const raw = sourceImageDataRef.current;
    if (!raw || saving) return;
    setSaving(true);
    try {
      const off = document.createElement("canvas");
      off.width = raw.width;
      off.height = raw.height;
      const octx = off.getContext("2d");
      if (!octx) return;
      octx.putImageData(raw, 0, 0);
      const dataUrl = off.toDataURL("image/png");
      await onSaveBase64(relPath, dataUrl);
      onDirtyChange(false);
    } finally {
      setSaving(false);
    }
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void handleSave();
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
        if (e.shiftKey) handleRedo();
        else handleUndo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [relPath]);

  const handlePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button === 1 || tool === "pan" || e.altKey) {
      panStartRef.current = {
        clientX: e.clientX,
        clientY: e.clientY,
        panX: pan.x,
        panY: pan.y,
      };
      e.currentTarget.setPointerCapture(e.pointerId);
      return;
    }
    const px = getPixelCoords(e.clientX, e.clientY);
    if (!px) return;

    const cur = sourceImageDataRef.current;
    if (!cur) return;

    if (tool === "eyedropper") {
      const idx = (px.y * cur.width + px.x) * 4;
      setColorHex(rgbToHex(cur.data[idx], cur.data[idx + 1], cur.data[idx + 2]));
      setAlphaVal(cur.data[idx + 3]);
      onToast(
        `Picked rgba(${cur.data[idx]}, ${cur.data[idx + 1]}, ${cur.data[idx + 2]}, ${cur.data[idx + 3]})`,
      );
      setTool("brush");
      return;
    }

    const [r, g, b] = hexToRgb(colorHex);
    const rgba: [number, number, number, number] =
      tool === "eraser" ? [0, 0, 0, 0] : [r, g, b, alphaVal];

    if (tool === "bucket") {
      floodFill(px.x, px.y, rgba);
      return;
    }

    pushUndoSnapshot();
    paintingRef.current = true;
    lastPixelRef.current = px;
    e.currentTarget.setPointerCapture(e.pointerId);
    drawBresenham(px.x, px.y, px.x, px.y, rgba);
  };

  const handlePointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const hover = getPixelCoords(e.clientX, e.clientY);
    setHoverPx(hover);

    if (panStartRef.current) {
      const dx = e.clientX - panStartRef.current.clientX;
      const dy = e.clientY - panStartRef.current.clientY;
      setPan({
        x: panStartRef.current.panX + dx,
        y: panStartRef.current.panY + dy,
      });
      return;
    }
    if (!paintingRef.current || !hover) return;
    const prev = lastPixelRef.current ?? hover;
    const [r, g, b] = hexToRgb(colorHex);
    const rgba: [number, number, number, number] =
      tool === "eraser" ? [0, 0, 0, 0] : [r, g, b, alphaVal];
    drawBresenham(prev.x, prev.y, hover.x, hover.y, rgba);
    lastPixelRef.current = hover;
  };

  const handlePointerUp = () => {
    paintingRef.current = false;
    lastPixelRef.current = null;
    panStartRef.current = null;
  };

  const scaledW = Math.max(1, imgWidth * zoom);
  const scaledH = Math.max(1, imgHeight * zoom);
  const showPixelGridOverlay = showGrid && zoom >= 5;
  const isGrassTexture = relPath.toLowerCase().includes("grass");

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="paint-toolbar">
        <div className="toolbar-group">
          <span className="edition-badge">
            {imgWidth}x{imgHeight} {asset.format}
          </span>
          {(
            [
              ["brush", "codicon-edit", "Pencil"],
              ["eraser", "codicon-clear-all", "Eraser"],
              ["bucket", "codicon-paintcan", "Flood Fill"],
              ["eyedropper", "codicon-color-mode", "Eyedropper"],
              ["pan", "codicon-move", "Pan"],
            ] as const
          ).map(([id, icon, label]) => (
            <button
              key={id}
              className={`vsc-btn small ${tool === id ? "" : "secondary"}`}
              title={label}
              onClick={() => setTool(id)}
            >
              <span className={`codicon ${icon}`} />
              {label}
            </button>
          ))}

          <input
            type="color"
            value={colorHex}
            onChange={(e) => setColorHex(e.target.value)}
            title="Primary Color"
            style={{ width: 28, height: 24, border: "none", background: "transparent", cursor: "pointer" }}
          />
          <label style={{ fontSize: 11, display: "flex", alignItems: "center", gap: 4 }}>
            α:{alphaVal}
            <input
              type="range"
              min={0}
              max={255}
              value={alphaVal}
              onChange={(e) => setAlphaVal(Number(e.target.value))}
              style={{ width: 64 }}
            />
          </label>
        </div>

        <div className="toolbar-group">
          <button
            className="icon-btn"
            title="Undo (Ctrl+Z)"
            disabled={!canUndo}
            onClick={handleUndo}
          >
            <span className="codicon codicon-discard" />
          </button>
          <button
            className="icon-btn"
            title="Redo (Ctrl+Shift+Z)"
            disabled={!canRedo}
            onClick={handleRedo}
          >
            <span className="codicon codicon-redo" />
          </button>
          <button
            className="vsc-btn secondary small"
            onClick={() => {
              setZoom(computeFitZoom(imgWidth, imgHeight));
              setPan({ x: 0, y: 0 });
            }}
          >
            Fit
          </button>
          <button
            className={`vsc-btn small ${showGrid ? "" : "secondary"}`}
            onClick={() => setShowGrid((s) => !s)}
          >
            Grid
          </button>
          <button
            className={`vsc-btn small ${alphaMask ? "" : "secondary"}`}
            onClick={() => setAlphaMask((s) => !s)}
          >
            Alpha
          </button>
          <select
            className="vsc-select"
            value={biomeTint}
            onChange={(e) => setBiomeTint(e.target.value as BiomeTintPreset)}
            title="Biome Tint Preview"
            style={{ padding: "3px 6px", fontSize: 11 }}
          >
            <option value="none">Tint: None</option>
            <option value="plains_grass">Plains Grass</option>
            <option value="oak_foliage">Oak Foliage</option>
            <option value="water">Cold Water</option>
          </select>

          <select
            className="vsc-select"
            defaultValue=""
            onChange={(e) => {
              const target = Number(e.target.value);
              e.target.value = "";
              if (target > 0) void onDownscale(relPath, target);
            }}
            style={{ padding: "3px 6px", fontSize: 11 }}
          >
            <option value="" disabled>
              Downscale...
            </option>
            <option value="16">Nearest → 16x16</option>
            <option value="32">Nearest → 32x32</option>
            <option value="64">Nearest → 64x64</option>
            <option value="128">Nearest → 128x128</option>
          </select>

          {isGrassTexture && (
            <button
              className="vsc-btn secondary small"
              onClick={() => void onCompositeGrass(relPath)}
              title="Composite opaque grass_side.tga with alpha cutout"
            >
              Grass TGA
            </button>
          )}

          <button
            className="vsc-btn secondary small"
            onClick={() => onOpenBlockbench(relPath)}
          >
            <span className="codicon codicon-cube" />
            Blockbench
          </button>

          <button
            className="vsc-btn small"
            disabled={saving}
            onClick={() => void handleSave()}
          >
            <span className="codicon codicon-save" />
            {saving ? "Saving..." : "Save PNG"}
          </button>
        </div>
      </div>

      <div
        ref={viewportRef}
        className="paint-viewport"
        style={{
          cursor:
            tool === "pan"
              ? "grab"
              : tool === "eyedropper"
                ? "copy"
                : "crosshair",
        }}
        onWheel={(e) => {
          e.preventDefault();
          const factor = e.deltaY < 0 ? 1.2 : 0.82;
          setZoom((z) =>
            Math.max(0.05, Math.min(64, Number((z * factor).toFixed(3)))),
          );
        }}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerUp}
      >
        <div
          style={{
            position: "relative",
            width: `${scaledW}px`,
            height: `${scaledH}px`,
            transform: `translate(${pan.x}px, ${pan.y}px)`,
            backgroundImage:
              "linear-gradient(45deg, #2b2b2b 25%, transparent 25%), linear-gradient(-45deg, #2b2b2b 25%, transparent 25%), linear-gradient(45deg, transparent 75%, #2b2b2b 75%), linear-gradient(-45deg, transparent 75%, #2b2b2b 75%)",
            backgroundSize: "16px 16px",
            backgroundPosition: "0 0, 0 8px, 8px -8px, -8px 0px",
            backgroundColor: "#1d1d1d",
            flexShrink: 0,
          }}
        >
          <canvas
            ref={canvasRef}
            width={imgWidth}
            height={imgHeight}
            className="pixelated-canvas"
            style={{
              width: "100%",
              height: "100%",
              display: "block",
            }}
          />
          {showPixelGridOverlay && (
            <div
              style={{
                position: "absolute",
                inset: 0,
                pointerEvents: "none",
                backgroundImage:
                  "linear-gradient(to right, rgba(255,255,255,0.12) 1px, transparent 1px), linear-gradient(to bottom, rgba(255,255,255,0.12) 1px, transparent 1px)",
                backgroundSize: `${zoom}px ${zoom}px`,
              }}
            />
          )}
        </div>

        <div
          style={{
            position: "absolute",
            left: 10,
            bottom: 8,
            background: "rgba(30,30,30,0.85)",
            border: "1px solid #3e3e42",
            borderRadius: 4,
            padding: "3px 8px",
            fontSize: 11,
            color: "#cccccc",
          }}
        >
          Zoom: {Math.round(zoom * 100)}%
          {hoverPx ? ` | Pixel: (${hoverPx.x}, ${hoverPx.y})` : ""}
        </div>
      </div>
    </div>
  );
};
