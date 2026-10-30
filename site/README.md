# nuprint.app

NuPrint's website: plain HTML, CSS and SVG, served by Cloudflare as a static-assets Worker. It's separate from the apps and has no framework.

```sh
cd site
npm ci
npm run build      # writes dist/
npm run preview    # build, then serve dist/ with wrangler dev
```

- `src/` holds the pages (`index`, `changelog`, `404`), the partials they share, `style.css` and `download.js`. `{{TAG}}`, `{{VERSION}}` and `{{DOWNLOAD}}` are filled in at build time.
- `build.mjs` names the latest published release (`NUPRINT_TAG`, otherwise the newest version in `CHANGELOG.md`), renders the changelog up to that release, and copies the self-hosted fonts and the app's icon.
- `.github/workflows/site.yml` deploys on pushes to `master` that touch `site/`, and `release.yml` calls it after a release is published. It needs the repository secrets `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.
- The mascot animations come from the design canvas. They loop with CSS only and stop for visitors who prefer reduced motion.

Commit site changes as `docs(site): …` or `ci(site): …` so they stay out of the app's release notes.
