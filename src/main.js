/**
 * Application shell: import packages, pick a recolour, look at it, export it.
 *
 * The Rust side owns every format decision; this file only asks for a scan, a
 * preview or an export and arranges the answers.
 */

import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { createViewer } from "./viewer.js";
import {
  applyStatic,
  count,
  currentLanguage,
  describeError,
  onLanguageChange,
  setLanguage,
  t,
} from "./i18n.js";
import "./style.css";

const dom = {
  list: document.getElementById("list"),
  status: document.getElementById("status"),
  dropHint: document.getElementById("drop-hint"),
  hud: document.getElementById("hud"),
  hudTitle: document.getElementById("hud-title"),
  hudMeta: document.getElementById("hud-meta"),
  swatches: document.getElementById("swatches"),
  strip: document.getElementById("swatch-strip"),
  swatchCount: document.getElementById("swatch-count"),
  tools: document.getElementById("viewer-tools"),
  busy: document.getElementById("busy"),
  exportAll: document.getElementById("export-all"),
  withResources: document.getElementById("with-resources"),
  destination: document.getElementById("destination"),
  destinationButton: document.getElementById("pick-destination"),
};

const viewer = createViewer(document.getElementById("canvas"));

/** One entry per imported package; `choice` is the swatch the user settled on. */
const packages = [];
let selected = -1;
let destination = localStorage.getItem("destination") || "";
/** Preview requests can overlap; only the newest one may reach the screen. */
let previewToken = 0;

function setBusy(on) {
  dom.busy.hidden = !on;
}

function say(message, tone = "") {
  dom.status.textContent = message;
  dom.status.dataset.tone = tone;
}

function refreshDestination() {
  dom.destination.hidden = !destination;
  if (destination) {
    const parts = destination.split(/[\\/]/).filter(Boolean);
    dom.destinationButton.textContent = parts.slice(-2).join(" / ") || destination;
    dom.destinationButton.title = destination;
  }
}

function renderList() {
  dom.list.textContent = "";
  dom.exportAll.disabled = packages.length === 0;

  packages.forEach((entry, index) => {
    const card = document.createElement("article");
    card.className = "item";
    card.setAttribute("role", "listitem");
    if (index === selected) card.classList.add("on");
    if (entry.warning && entry.meshes === 0) card.classList.add("dead");

    const preview = document.createElement("div");
    preview.className = "chip";
    if (entry.swatches.length) {
      const image = document.createElement("img");
      image.src = entry.swatches[entry.choice ?? 0].thumbnail;
      image.alt = "";
      preview.append(image);
    }
    const game = document.createElement("span");
    game.className = "tag";
    game.textContent =
      entry.game === "Sims 2" ? "S2" : entry.game === "Sims 3" ? "S3" : "S4";
    preview.append(game);

    const body = document.createElement("div");
    body.className = "body";
    const name = document.createElement("h3");
    name.textContent = entry.name;
    name.title = entry.path;
    const meta = document.createElement("p");
    meta.textContent = entry.meshes
      ? `${count(entry.triangles, "triangle")} · ${count(entry.swatches.length, "colour")}`
      : t("item.nomesh");
    body.append(name, meta);
    if (entry.warning) {
      const warning = document.createElement("p");
      warning.className = "warn";
      warning.textContent = t(`warn.${entry.warning}`);
      body.append(warning);
    }

    const remove = document.createElement("button");
    remove.className = "remove";
    remove.title = t("item.remove");
    remove.textContent = "×";
    remove.addEventListener("click", (event) => {
      event.stopPropagation();
      packages.splice(index, 1);
      if (selected === index) {
        selected = -1;
        viewer.clear();
        dom.hud.hidden = true;
        dom.swatches.hidden = true;
        dom.tools.hidden = true;
        dom.dropHint.hidden = packages.length > 0;
      } else if (selected > index) {
        selected -= 1;
      }
      renderList();
    });

    card.append(preview, body, remove);
    card.addEventListener("click", () => select(index));
    dom.list.append(card);
  });
}

function renderSwatches(entry) {
  dom.strip.textContent = "";
  const many = entry.swatches.length > 1;
  dom.swatches.hidden = !many;
  if (!many) return;

  dom.swatchCount.textContent = entry.guessed
    ? t("swatch.pick", { n: entry.swatches.length })
    : count(entry.swatches.length, "variant");

  entry.swatches.forEach((swatch, index) => {
    const button = document.createElement("button");
    button.className = "swatch";
    button.title = `${swatch.width}×${swatch.height}`;
    if (index === entry.choice) button.classList.add("on");
    const image = document.createElement("img");
    image.src = swatch.thumbnail;
    image.alt = "";
    button.append(image);
    button.addEventListener("click", () => {
      if (entry.choice === index) return;
      entry.choice = index;
      renderSwatches(entry);
      renderList();
      loadPreview(entry);
    });
    dom.strip.append(button);
  });
}

async function loadPreview(entry) {
  const token = ++previewToken;
  setBusy(true);
  try {
    const buffer = await invoke("preview", {
      path: entry.path,
      swatch: entry.swatches[entry.choice]?.id ?? null,
    });
    if (token !== previewToken) return;
    await viewer.show(buffer instanceof ArrayBuffer ? buffer : new Uint8Array(buffer).buffer);
    dom.dropHint.hidden = true;
    dom.tools.hidden = false;
    say("");
  } catch (error) {
    if (token !== previewToken) return;
    viewer.clear();
    say(describeError(error), "bad");
  } finally {
    if (token === previewToken) setBusy(false);
  }
}

function select(index) {
  const entry = packages[index];
  if (!entry) return;
  selected = index;
  renderList();

  dom.hud.hidden = false;
  dom.hudTitle.textContent = entry.name;
  const bits = [entry.game, count(entry.meshes, "mesh"), count(entry.triangles, "triangle")];
  if (entry.has_normals) bits.push(t("meta.normalmap"));
  dom.hudMeta.textContent = bits.join(" · ");

  renderSwatches(entry);
  if (entry.meshes === 0) {
    viewer.clear();
    dom.tools.hidden = true;
    say(entry.warning ? t(`warn.${entry.warning}`) : t("status.nothing"), "bad");
    return;
  }
  loadPreview(entry);
}

async function addPaths(paths) {
  if (!paths?.length) return;
  setBusy(true);
  say(t("status.reading"));
  try {
    const scanned = await invoke("scan_packages", { paths });
    if (!scanned.length) {
      say(t("status.none"), "bad");
      return;
    }
    let firstNew = packages.length;
    for (const entry of scanned) {
      const existing = packages.findIndex((p) => p.path === entry.path);
      entry.choice = 0;
      if (existing >= 0) {
        packages[existing] = entry;
        firstNew = Math.min(firstNew, existing);
      } else {
        packages.push(entry);
      }
    }
    renderList();
    say(t("status.loaded", { n: count(scanned.length, "package") }));
    if (selected < 0) select(firstNew);
  } catch (error) {
    say(describeError(error), "bad");
  } finally {
    setBusy(false);
  }
}

async function chooseDestination() {
  const picked = await open({ directory: true, title: t("action.destination") });
  if (!picked) return null;
  destination = picked;
  localStorage.setItem("destination", destination);
  document.getElementById("lang").addEventListener("click", (event) => {
  const next = event.target.closest("button")?.dataset.lang;
  if (next && next !== currentLanguage()) setLanguage(next);
});

// Switching language rebuilds everything the interface wrote itself.
onLanguageChange(() => {
  renderList();
  const entry = packages[selected];
  if (entry) {
    renderSwatches(entry);
    dom.hudMeta.textContent = [
      entry.game,
      count(entry.meshes, "mesh"),
      count(entry.triangles, "triangle"),
      ...(entry.has_normals ? [t("meta.normalmap")] : []),
    ].join(" · ");
  }
  say("");
});

setLanguage(currentLanguage());
applyStatic();
refreshDestination();
  return destination;
}

async function exportAll() {
  if (!packages.length) return;
  const target = destination || (await chooseDestination());
  if (!target) return;

  const usable = packages.filter((entry) => entry.meshes > 0);
  if (!usable.length) {
    say(t("status.notexportable"), "bad");
    return;
  }

  setBusy(true);
  let done = 0;
  let failed = 0;
  for (const entry of usable) {
    say(
      t("status.exporting", {
        done: done + failed + 1,
        total: usable.length,
        name: entry.name,
      }),
    );
    try {
      await invoke("export", {
        path: entry.path,
        swatch: entry.swatches[entry.choice]?.id ?? null,
        destination: target,
        withResources: dom.withResources.checked,
      });
      done += 1;
    } catch (error) {
      failed += 1;
      console.error(entry.name, error);
    }
  }
  setBusy(false);
  say(
    failed
      ? t("status.exportedPartial", { n: count(done, "object"), failed })
      : t("status.exported", { n: count(done, "object"), path: target }),
    failed ? "bad" : "good",
  );
}

document.getElementById("pick-files").addEventListener("click", async () => {
  const picked = await open({
    multiple: true,
    filters: [{ name: "Sims package", extensions: ["package"] }],
  });
  await addPaths(picked ? (Array.isArray(picked) ? picked : [picked]) : []);
});

document.getElementById("pick-folder").addEventListener("click", async () => {
  const picked = await open({ directory: true, multiple: true });
  await addPaths(picked ? (Array.isArray(picked) ? picked : [picked]) : []);
});

dom.destinationButton.addEventListener("click", chooseDestination);
dom.exportAll.addEventListener("click", exportAll);

dom.tools.addEventListener("click", (event) => {
  const tool = event.target.closest("button")?.dataset.tool;
  if (tool === "reset") viewer.reset();
  if (tool === "wire") event.target.classList.toggle("on", viewer.toggleWireframe());
  if (tool === "grid") event.target.classList.toggle("on", !viewer.toggleGrid());
});

// Tauri delivers real filesystem paths on drop, which the browser never would.
// Outside a Tauri window there is no webview to ask, and the rest of the
// interface should still come up rather than die on the way past.
try {
  getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type === "over") {
      document.body.classList.add("dragging");
    } else if (event.payload.type === "drop") {
      document.body.classList.remove("dragging");
      addPaths(event.payload.paths);
    } else {
      document.body.classList.remove("dragging");
    }
  });
} catch (error) {
  console.warn("drag and drop is unavailable outside the application", error);
}

document.getElementById("lang").addEventListener("click", (event) => {
  const next = event.target.closest("button")?.dataset.lang;
  if (next && next !== currentLanguage()) setLanguage(next);
});

// Switching language rebuilds everything the interface wrote itself.
onLanguageChange(() => {
  renderList();
  const entry = packages[selected];
  if (entry) {
    renderSwatches(entry);
    dom.hudMeta.textContent = [
      entry.game,
      count(entry.meshes, "mesh"),
      count(entry.triangles, "triangle"),
      ...(entry.has_normals ? [t("meta.normalmap")] : []),
    ].join(" · ");
  }
  say("");
});

setLanguage(currentLanguage());
applyStatic();
refreshDestination();
