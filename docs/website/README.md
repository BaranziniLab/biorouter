# Website documentation sources

Edit `shell.html` or a fragment under `pages/`, then run from the repository root:

```bash
node landing/scripts/assemble-docs.mjs
node landing/scripts/assemble-docs.mjs --check
node scripts/check-crew-manual.mjs
node landing/scripts/check-docs-privacy.mjs
```

The assembler writes `landing/docs.html`. Commit both the sources and the generated page. It refuses missing fragments and mismatched page roots.

`{{LATEST_VERSION}}` is filled from the published fallback in `landing/assets/landing-site-content.md`. `scripts/release.sh landing <version>` updates that fallback and reassembles the generated page from these sources. Release notes marked “Next release” need a content review when those features ship.
