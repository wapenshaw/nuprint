# nuprint.app

NuPrint's website: plain HTML, CSS and SVG, served by Cloudflare as a static-assets Worker. It's separate from the apps and has no framework.

```sh
cd site
npm ci
npm run build      # writes dist/
npm run preview    # build, then serve dist/ with wrangler dev
```

- `src/` holds the pages (`index`, `guide`, `changelog`, `404`), the partials they share, `style.css` and `download.js`. `{{TAG}}`, `{{VERSION}}` and `{{DOWNLOAD}}` are filled in at build time.
- `build.mjs` names the latest published release (`NUPRINT_TAG`, otherwise the newest version in `CHANGELOG.md`), renders the changelog up to that release, renders `docs/user-guide.md` from that release's tag as `/guide` (with its screenshots; links to other docs go to the tag on GitHub), and copies the self-hosted fonts and the app's icon. A local build needs the tag fetched (`git fetch --tags`); `NUPRINT_GUIDE=local npm run build` uses the guide on disk instead, to preview edits before a release.
- `.github/workflows/site.yml` deploys on pushes to `master` that touch `site/` or the app files the site copies (the icon and the screenshot used for link previews), and `release.yml` calls it after a release is published. It needs the repository secrets `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.
- The mascot animations come from the design canvas. They loop with CSS only and stop for visitors who prefer reduced motion.

Commit site changes as `docs(site): …` or `ci(site): …` so they stay out of the app's release notes.
