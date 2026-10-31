# nuprint.app

NuPrint's website: plain HTML, CSS and SVG, served by Cloudflare as a static-assets Worker. It's separate from the apps and has no framework.

```sh
cd site
npm ci
npm run build      # writes dist/
npm run preview    # build, then serve dist/ with wrangler dev
```

- `src/` holds the pages (`index`, `guide`, `changelog`, `404`), the partials they share, `style.css` and `download.js`. `{{TAG}}`, `{{VERSION}}` and `{{DOWNLOAD}}` are filled in at build time.
- `build.mjs` names the latest published release (`NUPRINT_TAG`, otherwise the newest version in `CHANGELOG.md`), renders the changelog up to that release, renders `docs/user-guide.md` from that release's tag as `/guide` (with its screenshots; links to other docs go to the tag on GitHub), copies the self-hosted fonts, and takes the app's icon and a screenshot from the same tag. CI passes the release's file names in `NUPRINT_ASSETS`, and the page offers the `.rpm` and AppImage only when that release has them; without the list, Linux gets the `.deb` alone. A local build needs the tag fetched (`git fetch --tags`); `NUPRINT_GUIDE=local npm run build` uses the guide, icon and screenshot on disk instead, to preview edits before a release.
- The build also writes `robots.txt` and `sitemap.xml`. Every page uses `src/og.png` as its link-preview card.
- `art/cards.html` draws that card and the images for posts about NuPrint, from the mascot and the app's screenshots. After `npm run build`, run `node art/render.mjs`, serve `dist/` and screenshot each `.card` at its own size (`#og` is the 1200 × 630 link preview).
- `.github/workflows/site.yml` deploys on pushes to `master` that touch `site/`, and `release.yml` calls it after a release is published. It needs the repository secrets `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.
- The mascot animations come from the design canvas. They loop with CSS only and stop for visitors who prefer reduced motion.

Commit site changes as `docs(site): …` or `ci(site): …` so they stay out of the app's release notes.
