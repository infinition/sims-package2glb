/**
 * The three.js side: one scene, reused for every object.
 *
 * Loading a package always replaces what is on screen, so the scene is built
 * once and its contents swapped. Only the model subtree is disposed between
 * loads -- lights, grid and environment outlive it.
 */

import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { RoomEnvironment } from "three/examples/jsm/environments/RoomEnvironment.js";

export function createViewer(canvas) {
  const renderer = new THREE.WebGLRenderer({
    canvas,
    antialias: true,
    alpha: false,
  });
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.05;

  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0x161616);

  // A room probe gives the soft, even light these objects were authored under,
  // and costs one render rather than a rig of lights to maintain.
  const pmrem = new THREE.PMREMGenerator(renderer);
  scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;

  const key = new THREE.DirectionalLight(0xffffff, 1.6);
  key.position.set(2.5, 4, 3);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0xffe0bc, 0.4);
  fill.position.set(-3, 1.5, -2);
  scene.add(fill);

  const grid = new THREE.GridHelper(4, 16, 0x4a423a, 0x2b2926);
  grid.material.transparent = true;
  grid.material.opacity = 0.55;
  scene.add(grid);

  const camera = new THREE.PerspectiveCamera(38, 1, 0.01, 200);
  camera.position.set(1.4, 1.1, 1.8);

  const controls = new OrbitControls(camera, canvas);
  controls.enableDamping = true;
  controls.dampingFactor = 0.08;
  controls.minDistance = 0.05;
  controls.maxDistance = 40;

  const root = new THREE.Group();
  scene.add(root);

  const loader = new GLTFLoader();
  let wireframe = false;
  let frame = 0;

  function disposeChildren() {
    root.traverse((node) => {
      if (!node.isMesh) return;
      node.geometry?.dispose();
      const materials = Array.isArray(node.material) ? node.material : [node.material];
      for (const material of materials) {
        if (!material) continue;
        for (const slot of ["map", "normalMap", "emissiveMap"]) {
          material[slot]?.dispose();
        }
        material.dispose();
      }
    });
    root.clear();
  }

  /** Put the object on the floor, centred, and aim the camera at it. */
  function frameObject() {
    const box = new THREE.Box3().setFromObject(root);
    if (box.isEmpty()) return;
    const size = box.getSize(new THREE.Vector3());
    const centre = box.getCenter(new THREE.Vector3());

    root.position.sub(new THREE.Vector3(centre.x, box.min.y, centre.z));
    const span = Math.max(size.x, size.y, size.z) || 1;

    grid.scale.setScalar(Math.max(1, span * 1.6) / 4);
    controls.target.set(0, size.y * 0.45, 0);
    const distance = span * 2.2;
    camera.position.set(distance * 0.62, size.y * 0.55 + span * 0.75, distance * 0.78);
    camera.near = span / 200;
    camera.far = span * 60;
    camera.updateProjectionMatrix();
    controls.update();
  }

  async function show(buffer) {
    const gltf = await loader.parseAsync(buffer, "");
    disposeChildren();
    gltf.scene.traverse((node) => {
      if (node.isMesh) {
        node.material.wireframe = wireframe;
        // Sims meshes carry vertex normals but not always a full tangent
        // frame; three.js derives one from the UVs when TANGENT is missing.
        node.material.needsUpdate = true;
      }
    });
    root.add(gltf.scene);
    frameObject();
  }

  function resize() {
    const { clientWidth: width, clientHeight: height } = canvas;
    if (!width || !height) return;
    if (canvas.width !== width || canvas.height !== height) {
      renderer.setSize(width, height, false);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
    }
  }

  function tick() {
    frame = requestAnimationFrame(tick);
    resize();
    controls.update();
    renderer.render(scene, camera);
  }
  tick();

  return {
    show,
    clear: disposeChildren,
    reset: frameObject,
    toggleWireframe() {
      wireframe = !wireframe;
      root.traverse((node) => {
        if (node.isMesh) node.material.wireframe = wireframe;
      });
      return wireframe;
    },
    toggleGrid() {
      grid.visible = !grid.visible;
      return grid.visible;
    },
    dispose() {
      cancelAnimationFrame(frame);
      disposeChildren();
      pmrem.dispose();
      renderer.dispose();
    },
  };
}
