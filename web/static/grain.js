// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The grain at the screen's density (grain.css; from todobem's grain.js). grain.svg is a filter
// the browser evaluates per device pixel (and WebKit on every repaint: too slow as a background),
// so the tile is drawn once at one pixel per CSS pixel and scaled to the density without
// smoothing (each speck a k×k block, k at most 2 on a touch screen: memory); the rasters become
// --grain as data: URLs (the CSP's img-src allows them; nothing is stored or sent). A window
// carried to a screen of another density is done again. A browser that draws the SVG without
// its filter gives a blank raster: no grain then.

const LAYERS = [{ size: 512, at: "0 0" }, { size: 571, at: "256px 128px" }];   // as grain.css lays them
const coarse = matchMedia("(pointer: coarse)");
const density = () => Math.max(1, Math.round(devicePixelRatio || 1));
const scale = () => (coarse.matches ? Math.min(2, density()) : density());   // a speck's block, in device pixels
let tile = null;

function load() {
  tile ??= new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("grain.svg did not load"));
    img.src = "grain.svg";
  });
  tile.catch(() => { tile = null; });
  return tile;
}

/** Whether a drawn tile holds specks at all (a filter not applied draws nothing). */
function specked(canvas) {
  const data = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
  let n = 0;
  for (let i = 3; i < data.length; i += 28) if (data[i]) n++;
  return n * 28 > data.length * 0.005;
}

/** The tile at `size` CSS px, scaled by k without smoothing, as a data: URL. */
function raster(img, size, k) {
  const base = document.createElement("canvas");
  base.width = base.height = size;
  base.getContext("2d").drawImage(img, 0, 0, size, size);
  if (!specked(base)) throw new Error("the tile drew without its specks");
  const up = document.createElement("canvas");
  up.width = up.height = size * k;
  const ctx = up.getContext("2d");
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(base, 0, 0, up.width, up.height);
  return up.toDataURL("image/png");
}

function apply() {
  matchMedia(`(resolution: ${devicePixelRatio || 1}dppx)`).addEventListener("change", apply, { once: true });
  const k = scale(), root = document.documentElement.style;
  load().then((img) => {
    if (k !== scale()) return;                     // moved meanwhile: its own apply comes
    const urls = LAYERS.map((l) => raster(img, l.size, k));
    root.setProperty("--grain", LAYERS.map((l, i) => `url("${urls[i]}") ${l.at} / ${l.size}px ${l.size}px`).join(", "));
  }).catch(() => root.removeProperty("--grain"));   // no grain: the chrome stays as it is
}

apply();
