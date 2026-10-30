// Builds nuprint.app into dist/.
//
// The pages name the latest *published* release: CI passes its tag in NUPRINT_TAG
// (a draft release is never "latest"), and a local build falls back to the newest
// version in CHANGELOG.md. The changelog page shows only versions up to that tag,
// so a release whose installers are still building never appears early.

import { copyFile, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { marked } from "marked";

const site = (path) => fileURLToPath(new URL(path, import.meta.url));
const repo = (path) => site(`../${path}`);
const dist = site("dist/");
const REPO_URL = "https://github.com/wapenshaw/nuprint";

/** CHANGELOG.md as `{ version, date, body }`, newest first (release-please's format). */
function parseChangelog(text) {
  return text
    .split(/^(?=## )/m)
    .slice(1)
    .map((section) => {
      const [heading, ...rest] = section.split("\n");
      const match = heading.match(/^## \[?(\d+\.\d+\.\d+)\]?(?:\([^)]*\))? \((\d{4}-\d{2}-\d{2})\)/);
      if (!match) throw new Error(`unrecognised changelog heading: ${heading}`);
      return { version: match[1], date: match[2], body: rest.join("\n").trim() };
    });
}

function compareVersions(a, b) {
  const [x, y] = [a, b].map((v) => v.split(".").map(Number));
  return x[0] - y[0] || x[1] - y[1] || x[2] - y[2];
}

const formatDate = (iso) =>
  new Intl.DateTimeFormat("en-GB", { day: "numeric", month: "long", year: "numeric", timeZone: "UTC" }).format(
    new Date(`${iso}T00:00:00Z`),
  );

function renderRelease({ version, date, body }) {
  const notes = body
    ? marked.parse(body)
    : '<p class="release-empty">Maintenance only: nothing you would notice.</p>';
  return `<section class="release" aria-labelledby="v${version}">
<div class="release-head">
<h2 id="v${version}"><a href="${REPO_URL}/releases/tag/v${version}">${version}</a></h2>
<time datetime="${date}">${formatDate(date)}</time>
</div>
${notes}
</section>`;
}

const releases = parseChangelog(await readFile(repo("CHANGELOG.md"), "utf8"));
const tag = process.env.NUPRINT_TAG || `v${releases[0].version}`;
if (!/^v\d+\.\d+\.\d+$/.test(tag)) throw new Error(`NUPRINT_TAG must look like v1.2.3, got ${tag}`);
const version = tag.slice(1);
const published = releases.filter((release) => compareVersions(release.version, version) <= 0);

const partial = (name) => readFile(site(`src/partials/${name}.html`), "utf8");
const [head, nav, footer] = await Promise.all(["head", "nav", "footer"].map(partial));

/** Puts the partials in first (they have holes of their own), then fills every hole once. */
function fill(template, values) {
  return template
    .replace(/\{\{(HEAD|NAV|FOOTER)\}\}/g, (_, key) => ({ HEAD: head, NAV: nav, FOOTER: footer })[key])
    .replace(/\{\{(\w+)\}\}/g, (hole, key) => {
      if (!(key in values)) throw new Error(`no value for ${hole}`);
      return values[key];
    });
}

const values = {
  TAG: tag,
  VERSION: version,
  DOWNLOAD: `${REPO_URL}/releases/download/${tag}`,
  RELEASES: published.map(renderRelease).join("\n"),
};

await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });

for (const page of ["index", "changelog", "404"]) {
  const template = await readFile(site(`src/${page}.html`), "utf8");
  await writeFile(`${dist}${page}.html`, fill(template, values));
}

// Copied as they are: the site's own files, the fonts (self-hosted, so a visit
// reaches no one but this site) and the app's icon and screenshot.
const copies = {
  "style.css": site("src/style.css"),
  "download.js": site("src/download.js"),
  _headers: site("src/_headers"),
  "fonts/bricolage-grotesque.woff2": site(
    "node_modules/@fontsource-variable/bricolage-grotesque/files/bricolage-grotesque-latin-wght-normal.woff2",
  ),
  "fonts/space-mono-400.woff2": site("node_modules/@fontsource/space-mono/files/space-mono-latin-400-normal.woff2"),
  "fonts/space-mono-700.woff2": site("node_modules/@fontsource/space-mono/files/space-mono-latin-700-normal.woff2"),
  "icon.svg": repo("apps/tauri/src/assets/app-icons/cube.svg"),
  "apple-touch-icon.png": repo("apps/tauri/src-tauri/icons/128x128@2x.png"),
  "og.png": repo("docs/images/app-text.png"),
};

for (const [to, from] of Object.entries(copies)) {
  await mkdir(dirname(`${dist}${to}`), { recursive: true });
  await copyFile(from, `${dist}${to}`);
}

console.log(`Built nuprint.app for ${tag} with ${published.length} releases in the changelog.`);
