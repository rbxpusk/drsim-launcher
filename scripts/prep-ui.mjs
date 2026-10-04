import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = path.join(root, 'ui-dist');
const site = 'https://deltarunesim.com/';

const kit = {
  'view.js': 'assets/updates/view.js',
  'fonts.json': 'assets/stats/fonts.json',
  'fnt_main.png': 'assets/fnt_main.png',
  'fnt_mainbig.png': 'assets/fnt_mainbig.png',
  'box_corner.png': 'assets/stats/box_corner.png',
  'box_top.png': 'assets/stats/box_top.png',
  'box_left.png': 'assets/stats/box_left.png',
  'heart.png': 'assets/stats/heart.png',
  'heart_small.png': 'assets/stats/heart_small.png',
  'logo.png': 'assets/IMAGE_LOGO_0.png',
  'snd_menumove.wav': 'assets/snd_menumove.wav',
  'snd_select.wav': 'assets/snd_select.wav',
  'cyber_sky.png': 'assets/site/launch/spr_bg_cyber_parallax_clouds_0.png',
  'cyber_city.png': 'assets/site/launch/spr_bg_cyber_parallax_buildings_0.png',
  'cyber_lights.png': 'assets/site/launch/spr_bg_cyber_parallax_buildings_lights_0.png',
};
// background layers only, the launcher works without them
const optional = new Set(['cyber_sky.png', 'cyber_city.png', 'cyber_lights.png']);

fs.rmSync(out, { recursive: true, force: true });
fs.mkdirSync(path.join(out, 'kit'), { recursive: true });
for (const f of fs.readdirSync(path.join(root, 'ui'))) fs.copyFileSync(path.join(root, 'ui', f), path.join(out, f));

for (const [name, rel] of Object.entries(kit)) {
  const res = await fetch(site + rel);
  if (!res.ok) {
    if (optional.has(name)) { console.warn(`[prep-ui] skipped ${name} (${res.status})`); continue; }
    throw new Error(`prep-ui: ${site + rel} answered ${res.status}`);
  }
  fs.writeFileSync(path.join(out, 'kit', name), Buffer.from(await res.arrayBuffer()));
}

const fontsPath = path.join(out, 'kit', 'fonts.json');
const fonts = JSON.parse(fs.readFileSync(fontsPath, 'utf8'));
fs.writeFileSync(fontsPath, JSON.stringify({ fnt_main: fonts.fnt_main, fnt_mainbig: fonts.fnt_mainbig }));

const count = fs.readdirSync(out, { recursive: true }).length;
console.log(`[prep-ui] ${count} files -> ui-dist`);
