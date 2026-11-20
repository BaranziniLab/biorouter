# Official Computer Use Reference Bundles

This archival pre-release preserves historical official `computer-use` plugin bundles for reverse engineering, version comparison, and reproducible research. These files are reference inputs and are not part of the Open Computer Use build or installation process.

## Downloads

| File | Size (bytes) | SHA-256 |
| --- | ---: | --- |
| [1.0.750.zip](https://github.com/iFurySt/open-computer-use/releases/download/reference-bundles/1.0.750.zip) | 13458495 | `7afe231e98ddb3b95030c8c56bb178a0217e98fcbb08abf9e1b467fd7ead5b9d` |
| [1.0.755.zip](https://github.com/iFurySt/open-computer-use/releases/download/reference-bundles/1.0.755.zip) | 13467875 | `cfabe3f41bfec1ea1d5f99ca2f5424eac98487e9474a099394c372d5df7a6460` |

Download both ZIP files and the attached `SHA256SUMS` into the same directory, then verify them:

```bash
shasum -a 256 -c SHA256SUMS
```

The versioned files use the fixed `reference-bundles` tag rather than the software release's `latest` URL. They are uploaded Release assets, so downloading them does not use Git LFS bandwidth. Existing filenames and the archival tag should be retained; future bundle versions should use new filenames.

See the [reference bundle documentation](https://github.com/iFurySt/open-computer-use/blob/main/docs/references/codex-computer-use-reverse-engineering/assets/official-bundles/computer-use/README.md) for download commands and maintenance details. Historical commits may still contain LFS pointers; this migration does not rewrite Git history or restore an already consumed monthly LFS allowance.
