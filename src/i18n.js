/**
 * Two languages, English first.
 *
 * Static text is tagged in the markup with `data-i18n`; anything built at
 * runtime asks `t()` for its wording. Counts are the one thing a flat
 * dictionary cannot express on its own, so plural forms are held as a pair and
 * the choice is made per language: English pluralises everything but one,
 * French keeps zero singular too.
 */

const DICTIONARY = {
  en: {
    "app.tagline": "Sims 3 and Sims 4 objects to glTF",
    "action.files": "Import packages",
    "action.folder": "Import a folder",
    "action.export": "Export",
    "action.resources": "Also extract raw resources",
    "action.destination": "Output folder",
    "drop.title": "Drop your <b>.package</b> files here",
    "drop.hint": "One file, several, or a whole folder.",
    "swatch.title": "Colours",
    "tool.reset": "Recentre",
    "tool.wire": "Wireframe",
    "tool.grid": "Grid",
    "item.remove": "Remove",
    "item.nomesh": "no mesh",
    "meta.normalmap": "normal map",
    "status.reading": "Reading packages...",
    "status.none": "no .package found",
    "status.nothing": "nothing to display",
    "status.notexportable": "no package can be exported",
    "swatch.pick": "{n} textures, pick one",
    "warn.no_mesh": "no usable mesh in this package",
    "warn.sims2_no_geometry": "Sims 2: textures extract, geometry not supported yet",
    "warn.external_materials": "materials point outside the package, choose a colour by hand",
    "error.no_mesh": "no usable mesh (MODL/MLOD)",
    "error.not_dbpf": "this file is not a DBPF package",
    "error.bad_index": "package index out of bounds",
    "error.no_dds_header": "missing DDS header",
    "error.bad_texture_size": "invalid texture size",
    plural: {
      triangle: ["triangle", "triangles"],
      colour: ["colour", "colours"],
      variant: ["variant", "variants"],
      mesh: ["mesh", "meshes"],
      package: ["package", "packages"],
      object: ["object", "objects"],
    },
    "status.exporting": "Exporting {done}/{total}, {name}",
    "status.exported": "{n} exported to {path}",
    "status.exportedPartial": "{n} exported, {failed} failed",
    "status.loaded": "{n} loaded",
  },
  fr: {
    "app.tagline": "Objets Sims 3 et Sims 4 vers glTF",
    "action.files": "Importer des packages",
    "action.folder": "Importer un dossier",
    "action.export": "Exporter",
    "action.resources": "Extraire aussi les ressources brutes",
    "action.destination": "Dossier de sortie",
    "drop.title": "Deposez vos <b>.package</b> ici",
    "drop.hint": "Un fichier, plusieurs, ou un dossier entier.",
    "swatch.title": "Coloris",
    "tool.reset": "Recadrer",
    "tool.wire": "Fil de fer",
    "tool.grid": "Grille",
    "item.remove": "Retirer",
    "item.nomesh": "aucun maillage",
    "meta.normalmap": "carte de normales",
    "status.reading": "Lecture des packages...",
    "status.none": "aucun .package trouve",
    "status.nothing": "rien a afficher",
    "status.notexportable": "aucun package exportable",
    "swatch.pick": "{n} textures, a choisir",
    "warn.no_mesh": "aucun maillage exploitable dans ce package",
    "warn.sims2_no_geometry": "Sims 2 : les textures sortent, la geometrie n'est pas encore geree",
    "warn.external_materials": "les materiaux pointent hors du package, coloris a choisir a la main",
    "error.no_mesh": "aucun maillage exploitable (MODL/MLOD)",
    "error.not_dbpf": "ce fichier n'est pas un package DBPF",
    "error.bad_index": "index du package hors limites",
    "error.no_dds_header": "en-tete DDS absent",
    "error.bad_texture_size": "dimensions de texture invalides",
    plural: {
      triangle: ["triangle", "triangles"],
      colour: ["coloris", "coloris"],
      variant: ["variante", "variantes"],
      mesh: ["maillage", "maillages"],
      package: ["package", "packages"],
      object: ["objet", "objets"],
    },
    "status.exporting": "Export {done}/{total}, {name}",
    "status.exported": "{n} exporte(s) vers {path}",
    "status.exportedPartial": "{n} exporte(s), {failed} en echec",
    "status.loaded": "{n} charge(s)",
  },
};

let language = localStorage.getItem("language") === "fr" ? "fr" : "en";
const listeners = new Set();

export function currentLanguage() {
  return language;
}

export function setLanguage(next) {
  language = next === "fr" ? "fr" : "en";
  localStorage.setItem("language", language);
  document.documentElement.lang = language;
  applyStatic();
  for (const listener of listeners) listener(language);
}

export function onLanguageChange(listener) {
  listeners.add(listener);
}

export function t(key, values) {
  let text = DICTIONARY[language][key] ?? DICTIONARY.en[key] ?? key;
  if (values) {
    for (const [name, value] of Object.entries(values)) {
      text = text.replaceAll(`{${name}}`, value);
    }
  }
  return text;
}

/** "3 triangles" in whichever language is on. */
export function count(n, word) {
  const forms = DICTIONARY[language].plural[word] ?? DICTIONARY.en.plural[word];
  const many = language === "fr" ? n > 1 : n !== 1;
  return `${n} ${many ? forms[1] : forms[0]}`;
}

/**
 * Errors travel from Rust as stable codes so the wording lives here. Anything
 * unrecognised is shown as it came, which is better than swallowing it.
 */
export function describeError(error) {
  const raw = String(error?.message ?? error ?? "").trim();
  const key = `error.${raw}`;
  const known = DICTIONARY[language][key] ?? DICTIONARY.en[key];
  return known ?? raw;
}

export function applyStatic() {
  for (const node of document.querySelectorAll("[data-i18n]")) {
    node.innerHTML = t(node.dataset.i18n);
  }
  for (const node of document.querySelectorAll("[data-i18n-title]")) {
    node.title = t(node.dataset.i18nTitle);
  }
  for (const node of document.querySelectorAll("[data-lang]")) {
    node.classList.toggle("on", node.dataset.lang === language);
  }
}
