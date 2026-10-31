// Builds nuprint.app into dist/.
//
// The pages name the latest *published* release: CI passes its tag in NUPRINT_TAG
// (a draft release is never "latest"), and a local build falls back to the newest
// version in CHANGELOG.md. The changelog page shows only versions up to that tag,
// so a release whose installers are still building never appears early. The user
// guide is read from that tag too, so it describes the app the download installs,
// not features merged since.

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { copyFile, mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { Marked, marked } from "marked";

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

/** GitHub's heading anchors, so links into the guide keep working on both sites. */
const slug = (text) =>
  text
    .toLowerCase()
    .replace(/[^\w\- ]/g, "")
    .replace(/ /g, "-");

/**
 * A file as it was at a release tag (CI fetches the tag before building), or as it
 * is on disk with NUPRINT_GUIDE=local, to preview guide edits before they ship.
 */
const atTag = (tag, path) =>
  process.env.NUPRINT_GUIDE === "local"
    ? readFileSync(repo(path))
    : execFileSync("git", ["show", `${tag}:${path}`], { cwd: repo(""), maxBuffer: 64 << 20 });

/** A PNG's size from its header, so the page doesn't jump as screenshots load. */
function pngSize(png) {
  return { width: png.readUInt32BE(16), height: png.readUInt32BE(20) };
}

/**
 * docs/user-guide.md at `tag` as `{ intro, contents, html, images }`: the paragraph
 * under its title, a link to each `##` section, and one card per section. Its
 * screenshots (from the same tag) are published beside the page; links to the other
 * docs go to that tag on GitHub, because only the guide is published here.
 */
function renderGuide(tag) {
  const images = new Map();
  const guide = new Marked({
    walkTokens(token) {
      if (token.type === "image") {
        const name = token.href.replace(/^images\//, "");
        images.set(name, atTag(tag, `docs/images/${name}`));
        token.href = `/images/${name}`;
      } else if (token.type === "link" && !/^(https?:|#|mailto:)/.test(token.href)) {
        const [file, hash] = token.href.split("#");
        token.href = file === "user-guide.md" ? `#${hash}` : `${REPO_URL}/blob/${tag}/docs/${token.href}`;
      }
    },
    renderer: {
      heading({ tokens, depth, text }) {
        return `<h${depth} id="${slug(text)}">${this.parser.parseInline(tokens)}</h${depth}>\n`;
      },
    },
  });

  const text = atTag(tag, "docs/user-guide.md").toString("utf8").replace(/\r\n/g, "\n");
  const [title, ...sections] = text.split(/^(?=## )/m);
  const intro = title.replace(/^# .*\n/, "").trim();
  let html = sections.map((section) => `<section class="guide-part">\n${guide.parse(section)}</section>`).join("\n");

  for (const [name, png] of images) {
    const { width, height } = pngSize(png);
    html = html.replaceAll(`src="/images/${name}"`, `src="/images/${name}" width="${width}" height="${height}" loading="lazy"`);
  }
  const contents = sections
    .map((section) => section.match(/^## (.*)/)[1])
    .map((heading) => `<a href="#${slug(heading)}">${heading}</a>`)
    .join("\n");
  return { intro: guide.parseInline(intro), contents, html, images };
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

const guide = renderGuide(tag);

const values = {
  TAG: tag,
  VERSION: version,
  DOWNLOAD: `${REPO_URL}/releases/download/${tag}`,
  RELEASES: published.map(renderRelease).join("\n"),
  GUIDE_INTRO: guide.intro,
  GUIDE_CONTENTS: guide.contents,
  GUIDE: guide.html,
};

// Empty dist/ rather than delete it: `wrangler dev` keeps watching the folder.
await mkdir(dist, { recursive: true });
for (const entry of await readdir(dist)) {
  await rm(`${dist}${entry}`, { recursive: true, force: true });
}

for (const page of ["index", "guide", "changelog", "404"]) {
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

await mkdir(`${dist}images`, { recursive: true });
for (const [name, png] of guide.images) {
  await writeFile(`${dist}images/${name}`, png);
}

console.log(`Built nuprint.app for ${tag} with ${published.length} releases in the changelog.`);
