# Releases

This folder covers shipping BioRouter to users: the pre-release QA script for the
auto-update flow, an optional local recipe for cross-compiling binaries for other
architectures, and the published per-version release notes. It is written for
maintainers cutting a release and for developers doing ad-hoc packaging or
architecture QA.

Come here when you are preparing, verifying, or documenting a shipped version. If you
are installing BioRouter rather than building it, start with
[installation](../getting-started/installation.md). If you are standing BioRouter up as
a server people reach through a browser, that is [deployment](../deployment/README.md) —
the same packages, started with `biorouter serve`. Note that the release pipeline itself
(`scripts/release.sh`, the `release-*` targets in the `Justfile`) is the authority on
how releases are actually cut; the cross-compilation guide here is explicitly *not*
that path.

## Documents in this folder

| Document | What it covers |
|---|---|
| [1.92.0 release record](v1.92.0-release-handoff.md) | Completed release: launch changes, distribution validation, publication evidence and the historical pause/continuation. |
| [Auto-update test checklist](auto-update-test-checklist.md) | The verification plan for the one-click "Restart & Update" flow on macOS and the assisted-download fallback on Windows and Linux. Sections B–H are the live pre-release QA script to work through each release; Section A is a completed evidence log frozen at the 1.86.0 cycle, most recently executed 2026-07-14. |
| [Privacy and workspace test checklist](privacy-and-workspace-test-checklist.md) | The manual pass for privacy tiers, institutional affiliation and workspace control. States what a FAILURE looks like for each row, because several of these fail by doing nothing visible. |
| [v1.89.0 visual review](v1.89.0-visual-review.md) | Findings from a screenshot-by-screenshot pass over the desktop GUI after the Astryx rebuild and the privacy-tiers merge — fourteen defects and nine taste items, each with the surface, the theme and the measurement that explains it. The class of failure the automated suite cannot see, because jsdom has no layout and never runs Tailwind. |
| [Cross-compiling locally with `cross`](local-cross-compilation.md) | An optional local-QA recipe for building and smoke-testing release binaries for other architectures using the [`cross`](https://github.com/cross-rs/cross) tool, including running the result inside a matching container. Current, but not how releases are cut. |

## Subdirectories

- [`notes/`](notes/README.md) — the published per-version release notes, running from
  `v1.75.2` (May 2026) through `v1.89.2` (August 20, 2026); not every point release in
  that range has a note. Each states its release date, links the repository,
  summarizes the headline changes, and lists the per-platform download artifacts and
  install steps. These are frozen records of what shipped, indexed by version in the
  folder's own README.

## Related documentation

- [Deployment](../deployment/README.md) — reaching Biorouter through a browser, on
  your own machine or on a shared host.
- [Installation and setup](../getting-started/installation.md) — the per-platform
  install paths an update has to replace cleanly.
- [Environment variables](../configuration/environment-variables.md) — reference for
  `BIOROUTER_UPDATE_FEED_URL` and the other updater knobs the checklist exercises.
- [Troubleshooting](../troubleshooting/README.md) — where to send a user whose install
  or update did not come out the way this folder describes.
