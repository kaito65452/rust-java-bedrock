import { useEffect, useRef, useState } from "react";
import * as THREE from "three";

export interface VoxelTesseractLogoProps {
  variant?: "hero" | "badge";
  size?: number;
  className?: string;
}

/**
 * Crisp minimalist monochrome Voxel Tesseract vector emblem (#FFFFFF / #E4E4E7).
 * Used for the top-left header badge and as an automatic fallback when WebGL is unavailable.
 */
function VoxelTesseractSvg({
  size,
  className,
}: {
  size: number;
  className?: string;
}) {
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
      {/* Subtle back edges of outer wireframe cube */}
      <g stroke="#FFFFFF" strokeOpacity="0.24" strokeWidth="2.2" strokeLinecap="round">
        <line x1="64" y1="14" x2="64" y2="70" />
        <line x1="64" y1="70" x2="22" y2="92" />
        <line x1="64" y1="70" x2="106" y2="92" />
      </g>

      {/* 8 Crisp Hyper-edges connecting outer cube to inner voxel core */}
      <g stroke="#E4E4E7" strokeOpacity="0.62" strokeWidth="2.2" strokeLinecap="round">
        <line x1="64" y1="14" x2="64" y2="41" />
        <line x1="22" y1="36" x2="45" y2="51" />
        <line x1="106" y1="36" x2="83" y2="51" />
        <line x1="22" y1="92" x2="45" y2="77" />
        <line x1="106" y1="92" x2="83" y2="77" />
        <line x1="64" y1="114" x2="64" y2="87" />
        <line x1="64" y1="58" x2="64" y2="61" strokeOpacity="0.78" />
        <line x1="64" y1="70" x2="64" y2="67" strokeOpacity="0.28" />
      </g>

      {/* Faceted monochrome white inner voxel core (1.0 / 0.72 / 0.45 opacity) */}
      <polygon points="64,41 83,51 64,61 45,51" fill="#FFFFFF" fillOpacity="1.0" />
      <polygon points="45,51 64,61 64,87 45,77" fill="#FFFFFF" fillOpacity="0.72" />
      <polygon points="83,51 64,61 64,87 83,77" fill="#FFFFFF" fillOpacity="0.45" />
      <g stroke="#FFFFFF" strokeOpacity="0.92" strokeWidth="1.5" strokeLinejoin="round" strokeLinecap="round">
        <polygon points="64,41 83,51 83,77 64,87 45,77 45,51" fill="none" />
        <line x1="45" y1="51" x2="64" y2="61" />
        <line x1="83" y1="51" x2="64" y2="61" />
        <line x1="64" y1="61" x2="64" y2="87" />
      </g>

      {/* Crisp pure white outer wireframe cube */}
      <g stroke="#FFFFFF" strokeWidth="3.4" strokeLinejoin="round" strokeLinecap="round">
        <polygon points="64,14 106,36 106,92 64,114 22,92 22,36" fill="none" />
        <line x1="22" y1="36" x2="64" y2="58" />
        <line x1="106" y1="36" x2="64" y2="58" />
        <line x1="64" y1="58" x2="64" y2="114" />
      </g>
    </svg>
  );
}

/**
 * Minimalist monochrome 3D/4D Voxel Tesseract logo rendered with Three.js.
 * - `variant="hero"`: Pure white monochrome hypercube with subtle pointer tilt + SVG fallback.
 * - `variant="badge"`: Crisp lightweight vector emblem for the top-left header.
 */
export function VoxelTesseractLogo({
  variant = "hero",
  size,
  className,
}: VoxelTesseractLogoProps) {
  const pixelSize = size ?? (variant === "hero" ? 86 : 18);
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

    // Pure neutral studio lighting for clean monochrome faceted shading (zero color tint)
    const ambient = new THREE.AmbientLight(0xffffff, 0.62);
    scene.add(ambient);

    const topKey = new THREE.DirectionalLight(0xffffff, 1.35);
    topKey.position.set(2.5, 4.5, 3.2);
    scene.add(topKey);

    const fillLight = new THREE.DirectionalLight(0xe4e4e7, 0.45);
    fillLight.position.set(-3.2, -1.5, 2.4);
    scene.add(fillLight);

    const rootGroup = new THREE.Group();
    scene.add(rootGroup);

    const OUTER_SIZE = 2.05;
    const INNER_SIZE = 0.94;
    const outerHalf = OUTER_SIZE / 2;
    const innerHalf = INNER_SIZE / 2;

    // Outer subtle frosted monochrome shell + crisp pure-white wireframe edges
    const outerBoxGeo = new THREE.BoxGeometry(OUTER_SIZE, OUTER_SIZE, OUTER_SIZE);
    const outerGlassMat = new THREE.MeshBasicMaterial({
      color: 0xffffff,
      transparent: true,
      opacity: 0.04,
      depthWrite: false,
    });
    const outerGlassMesh = new THREE.Mesh(outerBoxGeo, outerGlassMat);
    rootGroup.add(outerGlassMesh);

    const outerEdgesGeo = new THREE.EdgesGeometry(outerBoxGeo);
    const outerEdgesMat = new THREE.LineBasicMaterial({
      color: 0xffffff,
      transparent: true,
      opacity: 0.96,
    });
    const outerWireframe = new THREE.LineSegments(outerEdgesGeo, outerEdgesMat);
    rootGroup.add(outerWireframe);

    // Faceted translucent monochrome white inner voxel core
    const innerBoxGeo = new THREE.BoxGeometry(INNER_SIZE, INNER_SIZE, INNER_SIZE);
    const innerCoreMat = new THREE.MeshLambertMaterial({
      color: 0xffffff,
      transparent: true,
      opacity: 0.88,
      flatShading: true,
    });
    const innerCoreMesh = new THREE.Mesh(innerBoxGeo, innerCoreMat);
    rootGroup.add(innerCoreMesh);

    const innerEdgesGeo = new THREE.EdgesGeometry(innerBoxGeo);
    const innerEdgesMat = new THREE.LineBasicMaterial({
      color: 0xffffff,
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
      color: 0xe4e4e7,
      transparent: true,
      opacity: 0.62,
    });
    const hyperLines = new THREE.LineSegments(hyperGeo, hyperMat);
    rootGroup.add(hyperLines);

    const tmpInner = new THREE.Vector3();
    const startTime = performance.now();
    let smoothTiltX = 0;
    let smoothTiltY = 0;

    const animate = (now: number) => {
      if (disposed || !renderer) return;
      rafId = window.requestAnimationFrame(animate);
      const t = (now - startTime) * 0.001;

      smoothTiltX += (tiltTargetRef.current.x - smoothTiltX) * 0.08;
      smoothTiltY += (tiltTargetRef.current.y - smoothTiltY) * 0.08;

      rootGroup.rotation.y = t * 0.42 + 0.62 + smoothTiltY * 0.45;
      rootGroup.rotation.x = 0.44 + Math.sin(t * 0.6) * 0.1 + smoothTiltX * 0.35;
      rootGroup.rotation.z = Math.cos(t * 0.42) * 0.05;

      const pulse = 1 + Math.sin(t * 1.7) * 0.04;
      innerCoreMesh.scale.setScalar(pulse);
      innerCoreMesh.rotation.y = Math.sin(t * 0.8) * 0.15;
      innerCoreMesh.rotation.x = Math.cos(t * 0.68) * 0.12;
      innerCoreMesh.updateMatrix();

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
      renderer?.dispose();
      if (container.contains(canvas)) {
        container.removeChild(canvas);
      }
    };
  }, [variant, pixelSize, webglFailed]);

  if (variant === "badge" || webglFailed) {
    return <VoxelTesseractSvg size={pixelSize} className={className} />;
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
