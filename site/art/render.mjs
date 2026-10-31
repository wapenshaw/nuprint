// Assembles cards.html into dist/art/ so a browser can screenshot each card at its pixel
// size. Run `npm run build` first: the cards use the built site's stylesheet and fonts.
import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";

const site = (path) => new URL(`../${path}`, import.meta.url);
const repo = (path) => new URL(`../../${path}`, import.meta.url);

const text = async (path) => (await readFile(site(path), "utf8")).replace(/\r\n/g, "\n");
const nav = await text("src/partials/nav.html");
const index = await text("src/index.html");
const defs = nav.match(/<svg class="sr-defs"[\s\S]*?\n<\/svg>/)[0];
const icon = nav.match(/<svg viewBox="100 100 824 824"[^>]*>[\s\S]*?<\/svg>/)[0];
const stage = index.match(/<div class="stage">[\s\S]*?<\/svg>\n<\/div>\n<\/div>/)[0];

// The text the app's screenshot previews, cut from the screenshot itself (paper x 565–1085).
const previewSlip = `<div class="receipt">
<div class="zig"></div>
<div class="slip-crop" style="height:calc(100cqw * 0.4167 * 290 / 520)"><img src="app-text.png" alt="" style="width:calc(100cqw * 0.4167 * 1250 / 520);left:calc(100cqw * 0.4167 * -565 / 520);top:calc(100cqw * 0.4167 * -205 / 520)"></div>
</div>`;

const printer = (kind) =>
  kind === "receipt" ? stage : stage.replace(/<div class="receipt a-receipt">[\s\S]*?\n<\/div>\n<\/div>\n<\/div>/, `${previewSlip}\n</div>`);

const html = (await text("art/cards.html"))
  .replace("{{DEFS}}", defs)
  .replace(/\{\{ICON (\d+)\}\}/g, (_, size) => icon.replace(/width="40" height="40"/, `width="${size}" height="${size}"`))
  .replace(/\{\{PRINTER ([\w-]+)\}\}/g, (_, kind) => printer(kind));

await mkdir(site("dist/art/"), { recursive: true });
await writeFile(site("dist/art/index.html"), html);
for (const name of ["app-text.png", "app-picture-dark.png"]) {
  await copyFile(repo(`docs/images/${name}`), site(`dist/art/${name}`));
}
console.log("Wrote dist/art/index.html");
