import type { Edition, PackType } from "../types";

export function packTypeBadgeLabel(packType: PackType): string {
  switch (packType) {
    case "java_resource_pack":
      return "Java RP";
    case "bedrock_resource_pack":
      return "Bedrock RP";
    case "bedrock_addon":
      return "Bedrock Add-on";
  }
}

export interface McFormattedSpan {
  text: string;
  color: string;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strikethrough: boolean;
  obfuscated: boolean;
}

const MC_COLOR_CODES: Record<string, string> = {
  "0": "#000000",
  "1": "#0000AA",
  "2": "#00AA00",
  "3": "#00AAAA",
  "4": "#AA0000",
  "5": "#AA00AA",
  "6": "#FFAA00",
  "7": "#AAAAAA",
  "8": "#555555",
  "9": "#5555FF",
  a: "#55FF55",
  b: "#55FFFF",
  c: "#FF5555",
  d: "#FF55FF",
  e: "#FFFF55",
  f: "#FFFFFF",
  // Bedrock material / metallic extended color codes:
  g: "#DDD605",
  h: "#E3D4D1",
  i: "#CECACA",
  j: "#443A3B",
  m: "#971607",
  n: "#B4684D",
  p: "#DEB12D",
  q: "#47A036",
  s: "#2CBAA8",
  t: "#21497B",
  u: "#9A5CC6",
};

/** Parses Minecraft `§` section-sign formatting codes into styled spans. */
export function parseMinecraftFormatting(input: string): McFormattedSpan[] {
  const spans: McFormattedSpan[] = [];
  let color = "#CCCCCC";
  let bold = false;
  let italic = false;
  let underline = false;
  let strikethrough = false;
  let obfuscated = false;
  let buf = "";

  const flush = () => {
    if (buf.length > 0) {
      spans.push({
        text: buf,
        color,
        bold,
        italic,
        underline,
        strikethrough,
        obfuscated,
      });
      buf = "";
    }
  };

  for (let i = 0; i < input.length; i += 1) {
    if (input[i] === "§" && i + 1 < input.length) {
      const code = input[i + 1].toLowerCase();
      if (MC_COLOR_CODES[code]) {
        flush();
        color = MC_COLOR_CODES[code];
        bold = false;
        italic = false;
        underline = false;
        strikethrough = false;
        obfuscated = false;
        i += 1;
        continue;
      }
      if (code === "l") {
        flush();
        bold = true;
        i += 1;
        continue;
      }
      if (code === "o") {
        flush();
        italic = true;
        i += 1;
        continue;
      }
      if (code === "n") {
        flush();
        underline = true;
        i += 1;
        continue;
      }
      if (code === "m") {
        flush();
        strikethrough = true;
        i += 1;
        continue;
      }
      if (code === "k") {
        flush();
        obfuscated = true;
        i += 1;
        continue;
      }
      if (code === "r") {
        flush();
        color = "#CCCCCC";
        bold = false;
        italic = false;
        underline = false;
        strikethrough = false;
        obfuscated = false;
        i += 1;
        continue;
      }
    }
    buf += input[i];
  }
  flush();
  return spans;
}

export function fileIconClass(relPath: string, isDir: boolean, expanded = false): string {
  if (isDir) {
    return expanded ? "codicon-folder-opened" : "codicon-folder";
  }
  const lower = relPath.toLowerCase();
  if (lower.endsWith(".png") || lower.endsWith(".tga") || lower.endsWith(".jpg") || lower.endsWith(".webp")) {
    return "codicon-file-media";
  }
  if (lower.endsWith(".geo.json") || lower.endsWith(".bbmodel")) {
    return "codicon-cube";
  }
  if (lower.endsWith(".json") || lower.endsWith(".mcmeta") || lower.endsWith(".material")) {
    return "codicon-json";
  }
  if (lower.endsWith(".ogg") || lower.endsWith(".wav") || lower.endsWith(".fsb")) {
    return "codicon-unmute";
  }
  if (lower.endsWith(".lang") || lower.endsWith(".txt") || lower.endsWith(".md")) {
    return "codicon-file-text";
  }
  if (lower.endsWith(".molang")) {
    return "codicon-symbol-method";
  }
  if (lower.endsWith(".pstdr") || lower.endsWith(".js") || lower.endsWith(".ts")) {
    return "codicon-file-code";
  }
  if (lower.endsWith(".toml")) {
    return "codicon-settings-gear";
  }
  return "codicon-file";
}

export function isImageAsset(relPath: string): boolean {
  const lower = relPath.toLowerCase();
  return lower.endsWith(".png") || lower.endsWith(".tga");
}

export function isTextCodeAsset(relPath: string): boolean {
  const lower = relPath.toLowerCase();
  return (
    lower.endsWith(".json") ||
    lower.endsWith(".mcmeta") ||
    lower.endsWith(".lang") ||
    lower.endsWith(".txt") ||
    lower.endsWith(".md") ||
    lower.endsWith(".molang") ||
    lower.endsWith(".material") ||
    lower.endsWith(".properties") ||
    lower.endsWith(".pstdr") ||
    lower.endsWith(".toml") ||
    lower.endsWith(".js") ||
    lower.endsWith(".ts")
  );
}

export function isModelAsset(relPath: string): boolean {
  const normalized = relPath.toLowerCase().replace(/^\/+/, "");
  return (
    normalized.endsWith(".geo.json") ||
    normalized.endsWith(".bbmodel") ||
    normalized.endsWith(".jem") ||
    normalized.endsWith(".jpm") ||
    (normalized.endsWith(".json") &&
      (normalized.startsWith("models/") || normalized.includes("/models/")))
  );
}

export function computeResourceId(relPath: string, edition: Edition): string {
  const norm = relPath.replace(/\\/g, "/").replace(/^\/+/, "");
  if (edition === "java" && norm.startsWith("assets/")) {
    const parts = norm.split("/");
    if (parts.length >= 4 && parts[2] === "textures") {
      const rest = parts.slice(3).join("/").replace(/\.(png|tga)$/i, "");
      return `${parts[1]}:${rest}`;
    }
    if (parts.length >= 4 && parts[2] === "models") {
      const rest = parts.slice(3).join("/").replace(/\.json$/i, "");
      return `${parts[1]}:${rest}`;
    }
    if (parts.length >= 3) {
      return `${parts[1]}:${parts.slice(2).join("/")}`;
    }
  }
  return norm.replace(/\.(png|tga|geo\.json|json)$/i, "");
}

export function generateUuidV4(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    const v = c === "x" ? r : (r & 0x3) | 0x8;
    return v.toString(16);
  });
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

export function baseName(relPath: string): string {
  const parts = relPath.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || relPath;
}
