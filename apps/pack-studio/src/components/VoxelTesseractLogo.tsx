import { useEffect, useRef, useState } from "react";
import * as THREE from "three";

export interface VoxelTesseractLogoProps {
  variant?: "hero" | "badge";
  size?: number;
  className?: string;
}

/**
 * Crisp vector Voxel Tesseract emblem used for the top-left header badge
 * and as an automatic zero-dependency fallback whenever WebGL is unavailable.
 */
function VoxelTesseractSvg({
  size,
  animated,
  className,
}: {
  size: number;
  animated?: boolean;
  className?: string;
}) {
  const uid = animated ? "vt-hero" : "vt-badge";
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 128 128"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      aria-label="PackStudio Voxel Tesseract Logo"
      style={{ display: "block", flexShrink: 0 }}
    >
      <defs>
        <linearGradient id={`${uid}-outer`} x1="16" y1="16" x2="112" y2="112" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#7dd3fc" />
          <stop offset="55%" stopColor="#38bdf8" />
          <stop offset="100%" stopColor="#a78bfa" />
        </linearGradient>
        <linearGradient id={`${uid}-core-top`} x1="44" y1="42" x2="84" y2="62" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#6ee7b7" />
          <stop offset="100%" stopColor="#38bdf8" />
        </linearGradient>
        <linearGradient id={`${uid}-core-left`} x1="44" y1="52" x2="64" y2="86" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#22d3ee" />
          <stop offset="100%" stopColor="#2563eb" />
        </linearGradient>
        <linearGradient id={`${uid}-core-right`} x1="84" y1="52" x2="64" y2="86" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stopColor="#818cf8" />
          <stop offset="100%" stopColor="#4f46e5" />
        </linearGradient>
      </defs>

      {/* Outer translucent faces */}
      <polygon points="64,14 106,36 64,58 22,36" fill="#38bdf8" fillOpacity="0.14" />
      <polygon points="22,36 64,58 64,114 22,92" fill="#3b82f6" fillOpacity="0.1" />
      <polygon points="106,36 64,58 64,114 106,92" fill="#8b5cf6" fillOpacity="0.12" />

      {/* Back edges */}
      <g stroke="#38bdf8" strokeOpacity="0.32" strokeWidth="2.5" strokeLinecap="round">
        <line x1="64" y1="14" x2="64" y2="70" />
        <line x1="64" y1="70" x2="22" y2="92" />
        <line x1="64" y1="70" x2="106" y2="92" />
      </g>

      {/* 8 Hyper-edges connecting outer cube to inner core */}
      <g stroke="#c4b5fd" strokeOpacity="0.82" strokeWidth="2.4" strokeLinecap="round">
        <line x1="64" y1="14" x2="64" y2="41" />
        <line x1="22" y1="36" x2="45" y2="51" />
        <line x1="106" y1="36" x2="83" y2="51" />
        <line x1="22" y1="92" x2="45" y2="77" />
        <line x1="106" y1="92" x2="83" y2="77" />
        <line x1="64" y1="114" x2="64" y2="87" />
        <line x1="64" y1="58" x2="64" y2="61" />
        <line x1="64" y1="70" x2="64" y2="67" strokeOpacity="0.45" />
      </g>

      {/* Glowing inner voxel core */}
      <polygon points="64,41 83,51 64,61 45,51" fill={`url(#${uid}-core-top)`} />
      <polygon points="45,51 64,61 64,87 45,77" fill={`url(#${uid}-core-left)`} />
      <polygon points="83,51 64,61 64,87 83,77" fill={`url(#${uid}-core-right)`} />
      <g stroke="#e0f2fe" strokeOpacity="0.92" strokeWidth="2" strokeLinejoin="round" strokeLinecap="round">
        <polygon points="64,41 83,51 83,77 64,87 45,77 45,51" fill="none" />
        <line x1="45" y1="51" x2="64" y2="61" />
        <line x1="83" y1="51" x2="64" y2="61" />
        <line x1="64" y1="61" x2="64" y2="87" />
      </g>

      {/* Outer wireframe cube */}
      <g stroke={`url(#${uid}-outer)`} strokeWidth="3.6" strokeLinejoin="round" strokeLinecap="round">
        <polygon points="64,14 106,36 106,92 64,114 22,92 22,36" fill="none" />
        <line x1="22" y1="36" x2="64" y2="58" />
        <line x1="106" y1="36" x2="64" y2="58" />
        <line x1="64" y1="58" x2="64" y2="114" />
      </g>

      {/* Corner nodes */}
      <g fill="#e0f2fe">
        <circle cx="64" cy="14" r="3" />
        <circle cx="22" cy="36" r="3" />
        <circle cx="106" cy="36" r="3" />
        <circle cx="64" cy="58" r="3.2" />
        <circle cx="22" cy="92" r="3" />
        <circle cx="106" cy="92" r="3" />
        <circle cx="64" cy="114" r="3" />
      </g>
    </svg>
  );
}

/**
 * Interactive 3D/4D Voxel Tesseract logo rendered with Three.js.
 * - `variant="hero"`: Live WebGL animated hypercube with subtle pointer tilt + automatic SVG fallback.
 * - `variant="badge"`: Crisp lightweight vector emblem for the compact top-left header.
 */
export function VoxelTesseractLogo({
  variant = "hero",
  size,
  className,
}: VoxelTesseractLogoProps) {
  const pixelSize = size ?? (variant === "hero" ? 88 : 18);
  const mountRef = useRef<HTMLDivElement | null>(null);
  const tiltTargetRef = useRef({ x: 0, y: 0 });
  const [webglFailed, setWebglFailed] = useState(false);

  useEffect(() => {
    if (variant !== "hero" || webglFailed) return;
    const container = mountRef.current;
    if (!container) return;

    let renderer: THREE.WebGLRenderer | null = null;
    let rafId = 0;
    let disposed = false;

    try {
      renderer = new THREE.WebGLRenderer({
        alpha: true,
        antialias: true,
        powerPreference: "low-power",
      });
    } catch {
      setWebglFailed(true);
      return;
    }

    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    renderer.setPixelRatio(dpr);
    renderer.setSize(pixelSize, pixelSize);
    renderer.setClearColor(0x000000, 0);

    const canvas = renderer.domElement;
    canvas.style.width = `${pixelSize}px`;
    canvas.style.height = `${pixelSize}px`;
    canvas.style.display = "block";
    container.innerHTML = "";
    container.appendChild(canvas);

    const onContextLost = (ev: Event) => {
      ev.preventDefault();
      if (!disposed) setWebglFailed(true);
    };
    canvas.addEventListener("webglcontextlost", onContextLost, false);

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(34, 1, 0.1, 100);
    camera.position.set(0, 0.15, 5.15);

    const ambient = new THREE.AmbientLight(0xdbeafe, 0.9);
    scene.add(ambient);

    const keyLight = new THREE.PointLight(0x38bdf8, 4.2, 20);
    keyLight.position.set(3.2, 3.6, 4.0);
    scene.add(keyLight);

    const rimLight = new THREE.PointLight(0xa78bfa, 3.4, 20);
    rimLight.position.set(-3.4, -2.4, 3.2);
    scene.add(rimLight);

    const rootGroup = new THREE.Group();
    scene.add(rootGroup);

    const OUTER_SIZE = 2.05;
    const INNER_SIZE = 0.94;
    const outerHalf = OUTER_SIZE / 2;
    const innerHalf = INNER_SIZE / 2;

    // Outer translucent glass cube + crisp wireframe
    const outerBoxGeo = new THREE.BoxGeometry(OUTER_SIZE, OUTER_SIZE, OUTER_SIZE);
    const outerGlassMat = new THREE.MeshPhysicalMaterial({
      color: 0x38bdf8,
      emissive: 0x1d4ed8,
      emissiveIntensity: 0.25,
      transparent: true,
      opacity: 0.14,
      roughness: 0.18,
      metalness: 0.1,
      depthWrite: false,
    });
    const outerGlassMesh = new THREE.Mesh(outerBoxGeo, outerGlassMat);
    rootGroup.add(outerGlassMesh);

    const outerEdgesGeo = new THREE.EdgesGeometry(outerBoxGeo);
    const outerEdgesMat = new THREE.LineBasicMaterial({
      color: 0x7dd3fc,
      transparent: true,
      opacity: 0.95,
    });
    const outerWireframe = new THREE.LineSegments(outerEdgesGeo, outerEdgesMat);
    rootGroup.add(outerWireframe);

    // Glowing inner voxel core
    const innerBoxGeo = new THREE.BoxGeometry(INNER_SIZE, INNER_SIZE, INNER_SIZE);
    const innerCoreMat = new THREE.MeshStandardMaterial({
      color: 0x38bdf8,
      emissive: 0x22d3ee,
      emissiveIntensity: 0.65,
      roughness: 0.25,
      metalness: 0.35,
    });
    const innerCoreMesh = new THREE.Mesh(innerBoxGeo, innerCoreMat);
    rootGroup.add(innerCoreMesh);

    const innerEdgesGeo = new THREE.EdgesGeometry(innerBoxGeo);
    const innerEdgesMat = new THREE.LineBasicMaterial({
      color: 0xe0f2fe,
      transparent: true,
      opacity: 0.95,
    });
    const innerWireframe = new THREE.LineSegments(innerEdgesGeo, innerEdgesMat);
    innerCoreMesh.add(innerWireframe);

    // 8 Hyper-edges connecting corresponding outer and inner vertices
    const unitSigns: [number, number, number][] = [
      [-1, -1, -1],
      [1, -1, -1],
      [1, 1, -1],
      [-1, 1, -1],
      [-1, -1, 1],
      [1, -1, 1],
      [1, 1, 1],
      [-1, 1, 1],
    ];
    const hyperPositions = new Float32Array(unitSigns.length * 2 * 3);
    const hyperGeo = new THREE.BufferGeometry();
    hyperGeo.setAttribute(
      "position",
      new THREE.BufferAttribute(hyperPositions, 3),
    );
    const hyperMat = new THREE.LineBasicMaterial({
      color: 0xc4b5fd,
      transparent: true,
      opacity: 0.85,
    });
    const hyperLines = new THREE.LineSegments(hyperGeo, hyperMat);
    rootGroup.add(hyperLines);

    // Subtle corner nodes on the outer cube
    const nodeGeo = new THREE.SphereGeometry(0.055, 10, 10);
    const nodeMat = new THREE.MeshBasicMaterial({ color: 0xe0f2fe });
    const cornerNodes: THREE.Mesh[] = [];
    for (const [sx, sy, sz] of unitSigns) {
      const node = new THREE.Mesh(nodeGeo, nodeMat);
      node.position.set(sx * outerHalf, sy * outerHalf, sz * outerHalf);
      rootGroup.add(node);
      cornerNodes.push(node);
    }

    const tmpInner = new THREE.Vector3();
    const startTime = performance.now();
    let smoothTiltX = 0;
    let smoothTiltY = 0;

    const animate = (now: number) => {
      if (disposed || !renderer) return;
      rafId = window.requestAnimationFrame(animate);
      const t = (now - startTime) * 0.001;

      // Smooth pointer tilt interpolation
      smoothTiltX += (tiltTargetRef.current.x - smoothTiltX) * 0.08;
      smoothTiltY += (tiltTargetRef.current.y - smoothTiltY) * 0.08;

      // Outer cube gentle isometric rotation + pointer tilt
      rootGroup.rotation.y = t * 0.48 + 0.62 + smoothTiltY * 0.45;
      rootGroup.rotation.x = 0.44 + Math.sin(t * 0.65) * 0.12 + smoothTiltX * 0.35;
      rootGroup.rotation.z = Math.cos(t * 0.45) * 0.06;

      // Subtle 4D breathing/counter-twist on the inner voxel core
      const pulse = 1 + Math.sin(t * 1.9) * 0.055;
      innerCoreMesh.scale.setScalar(pulse);
      innerCoreMesh.rotation.y = Math.sin(t * 0.85) * 0.18;
      innerCoreMesh.rotation.x = Math.cos(t * 0.72) * 0.14;
      innerCoreMesh.updateMatrix();

      // Recompute the 8 hyper-edge endpoints to track the inner core vertices
      const posAttr = hyperGeo.getAttribute("position") as THREE.BufferAttribute;
      for (let i = 0; i < unitSigns.length; i += 1) {
        const [sx, sy, sz] = unitSigns[i];
        const base = i * 6;
        hyperPositions[base] = sx * outerHalf;
        hyperPositions[base + 1] = sy * outerHalf;
        hyperPositions[base + 2] = sz * outerHalf;

        tmpInner
          .set(sx * innerHalf, sy * innerHalf, sz * innerHalf)
          .applyMatrix4(innerCoreMesh.matrix);
        hyperPositions[base + 3] = tmpInner.x;
        hyperPositions[base + 4] = tmpInner.y;
        hyperPositions[base + 5] = tmpInner.z;
      }
      posAttr.needsUpdate = true;

      renderer.render(scene, camera);
    };

    rafId = window.requestAnimationFrame(animate);

    return () => {
      disposed = true;
      window.cancelAnimationFrame(rafId);
      canvas.removeEventListener("webglcontextlost", onContextLost);
      outerBoxGeo.dispose();
      outerGlassMat.dispose();
      outerEdgesGeo.dispose();
      outerEdgesMat.dispose();
      innerBoxGeo.dispose();
      innerCoreMat.dispose();
      innerEdgesGeo.dispose();
      innerEdgesMat.dispose();
      hyperGeo.dispose();
      hyperMat.dispose();
      nodeGeo.dispose();
      nodeMat.dispose();
      renderer?.dispose();
      if (container.contains(canvas)) {
        container.removeChild(canvas);
      }
    };
  }, [variant, pixelSize, webglFailed]);

  if (variant === "badge" || webglFailed) {
    return (
      <VoxelTesseractSvg
        size={pixelSize}
        animated={variant === "hero"}
        className={className}
      />
    );
  }

  return (
    <div
      ref={mountRef}
      className={className}
      style={{
        width: pixelSize,
        height: pixelSize,
        position: "relative",
        flexShrink: 0,
        cursor: "grab",
        filter: "drop-shadow(0 8px 22px rgba(56, 189, 248, 0.32))",
      }}
      onMouseMove={(e) => {
        const rect = e.currentTarget.getBoundingClientRect();
        const nx = ((e.clientX - rect.left) / rect.width - 0.5) * 2;
        const ny = ((e.clientY - rect.top) / rect.height - 0.5) * 2;
        tiltTargetRef.current = { x: ny, y: nx };
      }}
      onMouseLeave={() => {
        tiltTargetRef.current = { x: 0, y: 0 };
      }}
      title="PackStudio Voxel Tesseract"
    />
  );
}
