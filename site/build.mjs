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
const SITE_URL = "https://nuprint.app";

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
 * is on disk with NUPRINT_GUIDE=local, to preview edits to the guide or app images
 * before they ship.
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

const download = `${REPO_URL}/releases/download/${tag}`;

/**
 * The release's file names, which CI passes in NUPRINT_ASSETS, so the page offers the
 * .rpm and the AppImage only from the first release that has them. A build without the
 * list offers the .deb alone.
 */
const assets = (process.env.NUPRINT_ASSETS ?? "").split(/\s+/).filter(Boolean);
const rpm = assets.find((name) => /^NuPrint.*\.rpm$/.test(name));
const appImage = assets.find((name) => /^NuPrint.*\.AppImage$/.test(name));

const ticket = (file, name, detail) => `<a class="ticket lift" href="${download}/${file}">
<span class="ticket-stub"></span>
<span class="ticket-main"><span class="ticket-name">${name}</span><span class="ticket-file">${detail}</span></span>
<span class="ticket-icon"><svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 4v12M6 11l6 6 6-6M5 20h14"/></svg></span>
</a>`;

const values = {
  TAG: tag,
  VERSION: version,
  DOWNLOAD: download,
  // The AppImage runs on any distribution, so the hero button prefers it.
  LINUX_DOWNLOAD: `${download}/${appImage ?? `NuPrint_${version}_amd64.deb`}`,
  LINUX_TICKETS: [
    rpm && ticket(rpm, "Fedora and openSUSE", "64-bit · .rpm package"),
    appImage && ticket(appImage, "Any Linux", "64-bit · AppImage"),
  ]
    .filter(Boolean)
    .join("\n"),
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

// Copied as they are: the site's own files and the fonts (self-hosted, so a visit
// reaches no one but this site).
const copies = {
  "style.css": site("src/style.css"),
  "download.js": site("src/download.js"),
  // The link-preview card, drawn from art/cards.html (see README).
  "og.png": site("src/og.png"),
  _headers: site("src/_headers"),
  "fonts/bricolage-grotesque.woff2": site(
    "node_modules/@fontsource-variable/bricolage-grotesque/files/bricolage-grotesque-latin-wght-normal.woff2",
  ),
  "fonts/space-mono-400.woff2": site("node_modules/@fontsource/space-mono/files/space-mono-latin-400-normal.woff2"),
  "fonts/space-mono-700.woff2": site("node_modules/@fontsource/space-mono/files/space-mono-latin-700-normal.woff2"),
};

for (const [to, from] of Object.entries(copies)) {
  await mkdir(dirname(`${dist}${to}`), { recursive: true });
  await copyFile(from, `${dist}${to}`);
}

// The app's icon and screenshot come from the release tag, like the guide, so a merge
// that changes them waits for the release that ships it.
const appFiles = {
  "icon.svg": "apps/tauri/src/assets/app-icons/cube.svg",
  "apple-touch-icon.png": "apps/tauri/src-tauri/icons/128x128@2x.png",
  "images/app-text.png": "docs/images/app-text.png",
};

await mkdir(`${dist}images`, { recursive: true });
for (const [to, from] of Object.entries(appFiles)) {
  await writeFile(`${dist}${to}`, atTag(tag, from));
}

for (const [name, png] of guide.images) {
  await writeFile(`${dist}images/${name}`, png);
}

// The pages search engines should list. Cloudflare puts its content signals above this
// robots.txt; the Sitemap line still applies.
const pages = ["", "guide", "changelog"];
await writeFile(`${dist}robots.txt`, `User-agent: *\nAllow: /\n\nSitemap: ${SITE_URL}/sitemap.xml\n`);
await writeFile(
  `${dist}sitemap.xml`,
  `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${pages.map((page) => `<url><loc>${SITE_URL}/${page}</loc><lastmod>${published[0].date}</lastmod></url>`).join("\n")}
</urlset>
`,
);

console.log(`Built nuprint.app for ${tag} with ${published.length} releases in the changelog.`);
