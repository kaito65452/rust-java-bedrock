/* PackStudio <-> Blockbench 2-Way Bridge (Offline, Zero-Telemetry, Loopback-Only) */
(function () {
  "use strict";

  const params = new URLSearchParams(window.location.search);
  const studioPort = params.get("studio_port") || window.location.port;
  const token = params.get("token") || "";
  const apiBase = studioPort ? `http://127.0.0.1:${studioPort}` : window.location.origin;
  const wsUrl = studioPort
    ? `ws://127.0.0.1:${studioPort}/ws?token=${encodeURIComponent(token)}`
    : null;

  // 1. Offline & Zero-Telemetry Guard: block external network calls when running inside PackStudio
  const isLocalUrl = (input) => {
    try {
      const raw = typeof input === "string" ? input : (input && input.url) || "";
      if (!raw || raw.startsWith("/") || raw.startsWith("./") || raw.startsWith("../")) return true;
      if (raw.startsWith("data:") || raw.startsWith("blob:") || raw.startsWith("about:")) return true;
      const u = new URL(raw, window.location.href);
      return u.hostname === "127.0.0.1" || u.hostname === "localhost" || u.hostname === "[::1]";
    } catch (_) {
      return false;
    }
  };

  if (typeof window.fetch === "function") {
    const origFetch = window.fetch.bind(window);
    window.fetch = function (input, init) {
      if (!isLocalUrl(input)) {
        return Promise.resolve(
          new Response("{}", {
            status: 200,
            headers: { "Content-Type": "application/json", "X-PackStudio-Offline": "blocked" },
          })
        );
      }
      return origFetch(input, init);
    };
  }

  if (typeof window.XMLHttpRequest === "function") {
    const origOpen = XMLHttpRequest.prototype.open;
    const origSend = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function (method, url) {
      this.__packStudioBlocked = !isLocalUrl(url);
      if (this.__packStudioBlocked) {
        return;
      }
      return origOpen.apply(this, arguments);
    };
    XMLHttpRequest.prototype.send = function () {
      if (this.__packStudioBlocked) {
        try {
          Object.defineProperty(this, "readyState", { value: 4 });
          Object.defineProperty(this, "status", { value: 200 });
          Object.defineProperty(this, "responseText", { value: "{}" });
          if (typeof this.onload === "function") this.onload();
        } catch (_) {}
        return;
      }
      return origSend.apply(this, arguments);
    };
  }

  let activeModelPath = null;
  let activeFormatHint = null;
  let socket = null;
  let syncBadge = null;

  function setStatus(text, color) {
    if (!syncBadge) return;
    syncBadge.textContent = text;
    if (color) syncBadge.style.borderColor = color;
  }

  function showToast(msg) {
    try {
      if (window.Blockbench && typeof window.Blockbench.showQuickMessage === "function") {
        window.Blockbench.showQuickMessage(msg, 2400);
        return;
      }
    } catch (_) {}
    console.info("[PackStudio Bridge]", msg);
  }

  function waitForBlockbenchReady(cb) {
    let attempts = 0;
    const timer = setInterval(() => {
      attempts += 1;
      if (window.Blockbench && window.Blockbench.setup_successful && window.Codecs) {
        clearInterval(timer);
        cb();
      } else if (attempts > 300) {
        clearInterval(timer);
      }
    }, 100);
  }

  function dismissStartScreenAndEnterEdit() {
    try {
      if (window.StartScreen) {
        if (window.StartScreen.dialog && typeof window.StartScreen.dialog.close === "function") {
          window.StartScreen.dialog.close();
        }
        if (typeof window.StartScreen.close === "function") {
          window.StartScreen.close();
        }
        window.StartScreen.open = false;
      }
      if (window.Modes && window.Modes.options && window.Modes.options.edit && typeof window.Modes.options.edit.select === "function") {
        window.Modes.options.edit.select();
      }
    } catch (_) {}
  }

  function createProgrammaticProject(format) {
    if (!format) return false;
    if (typeof window.newProject === "function") {
      return Boolean(window.newProject(format));
    }
    if (typeof format.new === "function") {
      format.new();
      return true;
    }
    return false;
  }

  function openModelFromStudio(payload) {
    if (!payload) return;
    activeModelPath = payload.path || null;
    activeFormatHint = payload.format_hint || null;

    try {
      const lower = (payload.path || "").toLowerCase();
      const fileName = (payload.path || "model").split("/").pop() || "model";
      let loadedViaModelFile = false;

      if (
        payload.content &&
        (lower.endsWith(".bbmodel") ||
          activeFormatHint === "bbmodel" ||
          lower.endsWith(".geo.json") ||
          activeFormatHint === "bedrock" ||
          lower.endsWith(".json") ||
          activeFormatHint === "java_block")
      ) {
        if (typeof window.loadModelFile === "function") {
          try {
            window.loadModelFile({
              content: payload.content,
              path: payload.path || fileName,
              name: fileName,
            });
            loadedViaModelFile = true;
          } catch (loadErr) {
            console.warn("[PackStudio Bridge] loadModelFile fallback:", loadErr);
          }
        }

        if (!loadedViaModelFile) {
          const parsed = JSON.parse(payload.content);
          if (lower.endsWith(".bbmodel") || activeFormatHint === "bbmodel") {
            if (window.Codecs && window.Codecs.project) {
              if (typeof window.Codecs.project.load === "function") {
                window.Codecs.project.load(parsed, {
                  path: payload.path || fileName,
                  name: fileName,
                });
              } else {
                window.Codecs.project.parse(parsed, payload.path);
              }
            }
          } else if (lower.endsWith(".geo.json") || activeFormatHint === "bedrock") {
            createProgrammaticProject(window.Formats && window.Formats.bedrock);
            if (window.Codecs && window.Codecs.bedrock) {
              window.Codecs.bedrock.parse(parsed, payload.path);
            }
          } else if (lower.endsWith(".json") || activeFormatHint === "java_block") {
            createProgrammaticProject(window.Formats && window.Formats.java_block);
            if (window.Codecs && window.Codecs.java_block) {
              window.Codecs.java_block.parse(parsed, payload.path);
            }
          }
        }
      } else if (payload.textures && payload.textures.length > 0 && lower.endsWith(".png")) {
        // Open texture directly in Blockbench 2D/Paint mode
        createProgrammaticProject(
          (window.Formats && window.Formats.image) ||
            (window.Formats && window.Formats.free)
        );
      }

      if (Array.isArray(payload.textures) && window.Texture) {
        for (const tex of payload.textures) {
          if (!tex.data_url) continue;
          const t = new window.Texture({
            name: tex.name || (tex.path ? tex.path.split("/").pop() : "texture.png"),
            path: tex.path || "",
          }).fromDataURL(tex.data_url);
          t.__packStudioPath = tex.path || "";
          t.add(false);
        }
      }

      dismissStartScreenAndEnterEdit();

      if (window.Project && activeModelPath) {
        window.Project.__packStudioPath = activeModelPath;
      }
      showToast(`PackStudio opened: ${activeModelPath || "asset"}`);
      setStatus(`PackStudio: ${activeModelPath || "Connected"}`, "#89D185");
    } catch (err) {
      console.error("[PackStudio Bridge] Failed opening asset:", err);
      showToast(`PackStudio open error: ${err.message || err}`);
    }
  }

  function newBedrockGeometryFromStudio(payload) {
    const targetPath = (payload && payload.path) || "models/entity/custom_model.geo.json";
    const geoId = (payload && payload.identifier) || "geometry.custom_model";
    activeModelPath = targetPath;
    activeFormatHint = "bedrock";
    try {
      createProgrammaticProject(window.Formats && window.Formats.bedrock);
      dismissStartScreenAndEnterEdit();
      if (window.Project) {
        window.Project.geometry_name = geoId;
        window.Project.name = geoId.replace(/^geometry\./, "");
        window.Project.__packStudioPath = targetPath;
      }
      if (window.Cube) {
        const cube = new window.Cube({
          name: "body",
          from: [-4, 0, -4],
          to: [4, 8, 4],
        }).init();
        if (cube && typeof cube.select === "function") cube.select();
      }
      showToast(`New Bedrock Geometry: ${targetPath}`);
      setStatus(`PackStudio: ${targetPath}`, "#89D185");
    } catch (err) {
      console.error("[PackStudio Bridge] Failed creating Bedrock geometry:", err);
    }
  }

  function compileCurrentProject() {
    let modelContent = null;
    let modelPath =
      (window.Project && window.Project.__packStudioPath) ||
      activeModelPath ||
      null;

    try {
      if (window.Format && window.Format.codec && typeof window.Format.codec.compile === "function") {
        const formatId = window.Format.id || "";
        if (formatId === "image") {
          modelContent = null;
        } else {
          const compiled = window.Format.codec.compile();
          modelContent = typeof compiled === "string" ? compiled : JSON.stringify(compiled, null, 2);
          if (!modelPath) {
            const stem = ((window.Project && (window.Project.geometry_name || window.Project.name)) || "custom_model")
              .replace(/^geometry\./, "")
              .replace(/[^a-zA-Z0-9_\-]/g, "_")
              .toLowerCase();
            if (formatId.includes("bedrock")) {
              modelPath = `models/entity/${stem}.geo.json`;
            } else if (formatId.includes("java")) {
              modelPath = `assets/minecraft/models/block/${stem}.json`;
            } else {
              modelPath = `models/${stem}.bbmodel`;
            }
          }
        }
      }
    } catch (err) {
      console.error("[PackStudio Bridge] Compile model error:", err);
    }

    const textures = [];
    try {
      const list = (window.Texture && window.Texture.all) || [];
      for (const tex of list) {
        const dataUrl =
          typeof tex.getDataURL === "function"
            ? tex.getDataURL()
            : tex.source && String(tex.source).startsWith("data:")
            ? tex.source
            : null;
        if (!dataUrl) continue;
        const comma = dataUrl.indexOf(",");
        const pngBase64 = comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
        const texPath =
          tex.__packStudioPath ||
          (tex.path && !tex.path.startsWith("/") && !tex.path.includes(":") ? tex.path : null) ||
          `textures/entity/${(tex.name || "texture.png").replace(/[^a-zA-Z0-9._\-]/g, "_").toLowerCase()}`;
        textures.push({
          path: texPath.endsWith(".png") ? texPath : `${texPath}.png`,
          png_base64: pngBase64,
        });
      }
    } catch (err) {
      console.error("[PackStudio Bridge] Export textures error:", err);
    }

    return {
      type: "save",
      model_path: modelPath,
      model_content: modelContent,
      textures,
    };
  }

  async function syncToStudio() {
    const payload = compileCurrentProject();
    if (!payload.model_content && payload.textures.length === 0) {
      showToast("PackStudio: Nothing to sync yet.");
      return;
    }

    setStatus("PackStudio: Syncing...", "#CCA700");
    try {
      const resp = await window.fetch(
        `${apiBase}/api/save?token=${encodeURIComponent(token)}`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        }
      );
      const data = await resp.json().catch(() => ({}));
      if (resp.ok && data.ok !== false) {
        if (window.Project) window.Project.saved = true;
        const count = Array.isArray(data.saved_files) ? data.saved_files.length : 1;
        showToast(`Synced ${count} file(s) to PackStudio`);
        setStatus(`PackStudio: Synced (${count})`, "#89D185");
      } else {
        const errMsg = data.error || `HTTP ${resp.status}`;
        showToast(`PackStudio Sync Error: ${errMsg}`);
        setStatus("PackStudio: Sync Failed", "#F44747");
      }
    } catch (err) {
      showToast(`PackStudio Sync Failed: ${err.message || err}`);
      setStatus("PackStudio: Disconnected", "#F44747");
    }
  }

  function handleStudioCommand(cmd) {
    if (!cmd || !cmd.type) return;
    if (cmd.type === "open_model") {
      openModelFromStudio(cmd);
    } else if (cmd.type === "new_bedrock_geo") {
      newBedrockGeometryFromStudio(cmd);
    }
  }

  async function pollPendingCommands() {
    try {
      const resp = await window.fetch(
        `${apiBase}/api/commands?token=${encodeURIComponent(token)}`
      );
      if (!resp.ok) return;
      const cmds = await resp.json();
      if (Array.isArray(cmds)) {
        for (const cmd of cmds) handleStudioCommand(cmd);
      }
    } catch (_) {}
  }

  function connectBridge() {
    if (wsUrl && typeof window.WebSocket === "function") {
      try {
        socket = new WebSocket(wsUrl);
        socket.onopen = () => {
          setStatus("PackStudio: Connected", "#007ACC");
          pollPendingCommands();
        };
        socket.onmessage = (ev) => {
          try {
            handleStudioCommand(JSON.parse(ev.data));
          } catch (_) {}
        };
        socket.onclose = () => {
          setStatus("PackStudio: HTTP Bridge", "#CCA700");
        };
      } catch (_) {}
    }
    // Periodic HTTP command poll fallback so bridge works even if WS is interrupted
    pollPendingCommands();
    setInterval(pollPendingCommands, 1500);
  }

  function injectStudioUiAndHooks() {
    // Floating non-intrusive "Sync to Studio" pill in top-right header
    const bar = document.createElement("button");
    bar.id = "packstudio_sync_button";
    bar.type = "button";
    bar.textContent = "Sync to PackStudio (Ctrl+S)";
    bar.style.cssText = [
      "position: fixed",
      "right: 12px",
      "bottom: 28px",
      "z-index: 99999",
      "background: #1E1E1E",
      "color: #FFFFFF",
      "border: 1.5px solid #007ACC",
      "border-radius: 4px",
      "padding: 6px 12px",
      "font-family: monospace",
      "font-size: 12px",
      "cursor: pointer",
      "box-shadow: 0 3px 12px rgba(0,0,0,0.45)",
    ].join(";");
    bar.addEventListener("click", () => syncToStudio());
    document.body.appendChild(bar);
    syncBadge = bar;

    // Intercept Ctrl+S / Cmd+S to sync directly back to PackStudio
    window.addEventListener(
      "keydown",
      (ev) => {
        if ((ev.ctrlKey || ev.metaKey) && !ev.shiftKey && ev.key.toLowerCase() === "s") {
          ev.preventDefault();
          ev.stopPropagation();
          syncToStudio();
        }
      },
      true
    );

    // Register a Blockbench BarItems action if available
    try {
      if (window.Action) {
        new window.Action("packstudio_sync", {
          name: "Sync to PackStudio",
          description: "Save model and textures directly back to PackStudio project",
          icon: "sync",
          click: () => syncToStudio(),
        });
      }
      if (window.BarItems && window.BarItems.save_project && window.BarItems.save_project.click) {
        const origClick = window.BarItems.save_project.click;
        window.BarItems.save_project.click = function () {
          syncToStudio();
          return origClick.apply(this, arguments);
        };
      }
    } catch (_) {}

    window.PackStudioBridge = {
      syncToStudio,
      openModelFromStudio,
      newBedrockGeometryFromStudio,
    };
  }

  waitForBlockbenchReady(() => {
    injectStudioUiAndHooks();
    connectBridge();
  });

  // postMessage zoom/density listener — accepts messages from same-origin loopback or Tauri WebView
  window.addEventListener("message", function (event) {
    const origin = event.origin;
    if (
      origin !== window.location.origin &&
      origin !== "null" &&
      origin !== ""
    ) {
      return;
    }

    let data;
    try {
      data = typeof event.data === "string" ? JSON.parse(event.data) : event.data;
    } catch (_) {
      return;
    }
    if (!data || !data.type) return;

    if (data.type === "ps_set_zoom") {
      const raw = Number(data.zoom);
      if (!isFinite(raw)) return;
      const zoom = Math.min(1.5, Math.max(0.5, raw));
      document.documentElement.style.zoom = String(zoom);
      document.documentElement.style.fontSize = (zoom * 16) + "px";

    } else if (data.type === "ps_set_compact") {
      const compact = Boolean(data.compact);
      if (compact) {
        document.documentElement.classList.add("ps-compact");
        document.documentElement.style.setProperty("--toolbar-height", "32px");
        document.documentElement.style.setProperty("--status-bar-height", "20px");
      } else {
        document.documentElement.classList.remove("ps-compact");
        document.documentElement.style.removeProperty("--toolbar-height");
        document.documentElement.style.removeProperty("--status-bar-height");
      }

    } else if (data.type === "ps_reset_zoom") {
      document.documentElement.style.zoom = "";
      document.documentElement.style.fontSize = "";
      document.documentElement.classList.remove("ps-compact");
      document.documentElement.style.removeProperty("--toolbar-height");
      document.documentElement.style.removeProperty("--status-bar-height");
    }
  });

  // Convenience helpers on window.__ps
  window.__ps = window.__ps || {};
  window.__ps.setZoom = (z) =>
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "ps_set_zoom", zoom: z },
        origin: window.location.origin,
      })
    );
  window.__ps.resetZoom = () =>
    window.dispatchEvent(
      new MessageEvent("message", {
        data: { type: "ps_reset_zoom" },
        origin: window.location.origin,
      })
    );
})();
