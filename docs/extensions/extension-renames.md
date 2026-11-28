# Extension id renames

> **What this is.** How Biorouter renames an installed extension's id without breaking the chats, workflows, permissions and privacy tier that name it: the rename map, what the daemon moves once at start, what every other store resolves when it is read, the extension updater's rules, and the later pull request that moves BAAM to the new id.
> **Status:** Current. Added 2026-11-28 with the one rename made so far, `gptragent` to `gptr` (owner approval of 2026-11-28). The Biorouter half ships in the first release that carries the map, called R on this page; the BAAM half waits for the gptr turn-on pull request described at the end.
> **Audience:** developers working on extensions, the BAAM catalog or the extension updater, and reviewers of a rename.

An extension's id is the `name_to_key` key of its config entry's name (whitespace removed, lowercased). The one id is four things at once: the prefix of every tool the extension offers (`gptr__r`), the key of its entry in `config.yaml`, the folder it is installed in (`<config dir>/extensions/<id>/`) and the key of its provenance record. Saved chats, workflows, recipes, subagent lists, transcripts, skills, `permission.yaml`, hooks and managed policy all keep copies of the id or of its tool names. A rename therefore cannot be a search and replace: most of those stores are not Biorouter's to rewrite, and some of them, such as a transcript, are records that must keep the name they were written with.

## The rename map

[`crates/biorouter/src/config/extension_renames.json`](../../crates/biorouter/src/config/extension_renames.json) is the one table, `{"version": 1, "renames": [{"from", "to", "note"}]}`. Both names are stable keys (lowercase letters, digits, `_` and `-`), a name is renamed at most once, and there are no chains: a current id is never also an old one, so an extension renamed twice is renamed straight to its newest id. A row is never removed, because an old transcript, workflow or permission names the old id for as long as it exists.

| Old id | New id | Since | Tools now | Display name |
| --- | --- | --- | --- | --- |
| `gptragent` | `gptr` | 2026-11-28 | `gptr__sessions`, `gptr__inspect`, `gptr__r` | gptr Agent (unchanged) |

Four readers use the file, and only the first reads it compiled in:

| Reader | What it does with the map |
| --- | --- |
| [`crates/biorouter/src/config/extension_renames.rs`](../../crates/biorouter/src/config/extension_renames.rs) | Reads the table with `include_str!`, validates it in its tests, and is the API every Rust caller uses: `rename_of`, `current_key`, `all_keys`, `current_tool_name`, `tool_name_aliases`, `project_config` and `project_configs`. |
| `landing/scripts/build-registry.mjs` | Emits the other name of a renamed private extension into the compiled private set. |
| `landing/scripts/check-catalog.mjs` | Holds the BAAM catalog rows to the map (see [Rename an extension id](baam-catalog.md#rename-an-extension-id)). |
| [`ui/desktop/src/utils/extensionRenames.ts`](../../ui/desktop/src/utils/extensionRenames.ts) | The desktop's mirror, pinned to the JSON by `extensionRenames.test.ts`, because desktop code does not import files from outside `ui/desktop`. Edit the two together. |

## What moves at start

Three stores describe what is installed now, and the daemon moves them once: the provenance record, the install folder and the config entry. The move is `extension_rename::migrate_on_start` in `crates/biorouter/src/extension_rename.rs`, called by `biorouterd agent` beside the retired lead and worker migration, and by nothing else. As with the app folder move on Windows, only the daemon moves anything: the command line tool reads an unmoved install correctly through the projection below, and keeping the move out of it means a newer standalone CLI never moves a folder that an older desktop daemon's saved chats still point at.

For each rename the move runs in this order, under the install locks of both ids, and writes a journal (`<config dir>/extension-renames/<from>.json`) before it changes anything:

1. **Provenance first.** The record keyed by the old id is written again under the new id, keeping its registry id (`gptragent`: a BAAM id never moves), source, digest and time, with the new folder as its install directory. It gets a fresh install id, not the old one: the provenance store resolves which record of a key is current through pointers that name an install id, and one install id under two keys would let either record hide the other while both exist. A local install has no record to copy, so it is given one (see [Builds older than the rename](#builds-older-than-the-rename)). A record already under the new id decides the rest: a marketplace record of the same package (same registry id, with a source address) is kept; an identity record (same registry id, no source address, left by a config write that could not tell the old record named the folder, or a local install's own from before the move) is replaced by the copy, so the marketplace record's source and digest are not lost and the folder is the moved one; a record of another package (another registry id) is kept.
2. **Folder.** One `rename` of `<extensions>/<from>` to `<extensions>/<to>`, atomic within one directory, made only when a config entry of either id runs from the folder. A folder no entry runs from (an install parked at its credential step, whose claim names the folder, or what an uninstall left behind, perhaps with its record) is left where it is, and a record that names it keeps naming it. The Python environment moves with the folder and is synced again in the new folder off the start path, because `uv sync` writes absolute paths into it; `uv run` syncs once more on the first start if that failed.
3. **Config.** One atomic `config.yaml` write replaces the entry with its projection under the new key, in the same position, keeping `enabled`, `envs`, `env_keys`, `timeout` and `available_tools`.
4. **Retire the old record**, then remove the journal. The old record goes only when the record under the new id is the same package (same registry id, and a source address whenever the old record had one). Beside another package's record it stays, because it is the only one that says where this install came from, and the move then runs again, changing nothing, at each start.

Every state between two steps runs correctly, which is the crash-safety argument: after step 1 the extension is private under both keys, after step 2 every reader projects the old config onto the moved folder, and after step 3 the compiled private set still names both ids. A failed step is never rolled back; the journal makes the next start finish the move. When an install holds either lock, the move waits for the next start.

The move changes nothing, and logs one warning, when it cannot finish without breaking something: both ids are configured, either id is configured twice, another extension's entry sits under the new key, another extension runs from the old folder, an entry names the old folder in a way the move cannot rewrite (a path inside it, or the folder under another spelling such as a symlinked base), or both folders exist while the configuration still runs from the old one. Both names stay private by name in every such state, and the projection leaves the pair as it is. Both folders existing is not by itself a conflict: when the configuration already runs from the new folder, the config and the record move and the unused old folder is left in place and named in the log.

### Builds older than the rename

An older Biorouter build can share the config folder after the move (a terminal CLI that lags the desktop app, or an older desktop beside a newer one). It has no projection and no `gptr` in its compiled private set. Two things follow.

- **The tier holds, through a provenance record.** Every build since provenance records were added unions the registry ids of the records an entry's key finds, so a record keyed `gptr` that names the registry id `gptragent`, which every such build has in its private set, keeps the entry private there. A marketplace install has that record already (step 1 moves it). A local install (`biorouter extension install ./gptragent.brxt`, a double-clicked bundle, the desktop's file install) records none, so the move gives it one in step 1, and every config write gives one to an entry under the new id that no record names (`record_identities_of_renamed_installs`, called when `config.yaml`'s extensions are saved, which every installer and every write of the projection passes through, and called before the file is written, so no reader sees `gptr` without its record; a record whose write fails is written by the next save or the next start). The record has no source address, so a marketplace delete never takes it for a marketplace package, and a record can only raise a tier.
- **Routing is the accepted cost.** A saved chat whose roster still names the old folder cannot start gptr Agent in the older build until it is upgraded. A new chat there works, because the config entry already names the new folder.

## What resolves at read

Every other store keeps the name it was written with and is resolved through the map when it is read. Nothing stored is rewritten.

- **Configs** go through `project_config`, which gives a config under an old id its new name (so its tools carry the new prefix) and, once the folder has moved, points an argument that names the old install folder (compared as a path, so a trailing separator is the same folder) at the new one. It runs where configs are loaded: the `config.yaml` extension map, a saved chat's roster (`enabled_extensions/v0`) and a subagent's roster, and `ExtensionManager::add_extension_with_origin`, which every add passes through, workflows, recipes and the CLI included. A roster that already holds the new id keeps an old-id entry as it is, so no roster ever holds two configs with one key.
- **Tool names** are resolved twice. `offered_tool_for_name` (`agents/reply_parts.rs`) maps a model's call to `gptragent__r` onto `gptr__r` only when `gptr__r` is offered to that chat, never by consulting the full roster, so a hidden private tool's name cannot reach a public model. `prefixed_tool_name` at the top of `dispatch_tool_call`, the choke point every tool call passes through, maps it when the running manager holds the new id and not the old one.
- **Judges of tool names** test every spelling (`tool_name_aliases`): `permission.yaml` and smart approve, a session's always-confirm list, hook matchers, managed settings and managed security policy globs. A rule written for `gptragent__r`, a deny included, applies to `gptr__r`. Writing a permission for the current name removes that tool's old spellings from every list, so the newest choice wins.
- **Extension names given as strings** resolve with `current_key`: a subagent spawn's extension lists, `manage_extensions`, `workspace_set_tools`, the manager's remove and enabled checks, and the extension manager's tool that removes an installed package. That tool also takes `extensions/gptragent` as the folder of a `gptr` entry that still runs from it (the move has not run yet, or never will on a machine that only runs the command line tool), unless an entry of the old id is configured beside it, and it removes the provenance record still keyed `gptragent` that names that folder (`removal_provenance`), so a later reinstall does not leave two records for one registry id.
- **Provenance records keyed by an old id** are found for their current entry: `delete_extension_package` matches a record keyed `gptragent` to the configured `gptr` entry when no `gptragent` entry is configured (`configured_entry_of_record`), so a BAAM package the move has not carried can still be deleted.
- **Both ids configured** is the conflict the move leaves alone, with both entries loaded. Removing one of them removes only that one: its saved-chat roster entries and its `<id>__` permission choices go, and the other id's stay (`prune_extension_from_sessions` and `PermissionManager::remove_extension` compare exact ids while another id of the extension is configured). A permission choice is matched by its full `<id>__` prefix, so removing `gptr` never takes the choices of an extension whose name merely begins with `gptr`.
- **`available_tools` lists** hold bare tool names (`r`, not `gptragent__r`), so a rename does not touch them.

## Privacy

Both names of a renamed private extension are in the compiled private set (`crates/biorouter/src/privacy/registry_private.rs`, generated from the BAAM page). `build-registry.mjs` emits the second name from the rename map with the private card's affiliation, the closed list `EXPECTED_PRIVATE_EXTENSIONS` names both, and the desktop's `PRIVATE_EXTENSION_KEYS` lists both and is pinned to the Rust file. So the map is not a privacy input: a lookup that forgets to resolve a name still classifies it Private, and what it can lose is routing, never the tier. The move keeps this too: provenance is written under the new id before the old record is retired, and the registry id never moves, so a chat that held `gptragent` keeps its tier and its grants. Builds older than the rename keep the tier through the provenance record described in [Builds older than the rename](#builds-older-than-the-rename). The full design is in [Privacy tiers](../security/privacy-tiers.md).

Classification is by name, so an unrelated server someone adds by hand as `gptr` is classified Private, exactly as a hand-added `cdwagent` is: a public chat is refused it and nothing is lowered. The new id is one more name such a server can take, and a config write gives it the raise-only record above too. The move never folds a `gptragent` install into it: while an entry of the new id sits beside an entry of the old one, or another extension's entry holds the key `gptr`, the move changes nothing.

## Installers

An installer renames only an id the map renames. `gptragent.brxt` installs as `gptr`: its folder, its config name and its provenance record use the new id, while its entry point stays the bundle's own (`gptragent`), because the config's `uv run --directory <folder> <entry point>` must name a script the bundle has. Any other bundle keeps its manifest name exactly as written, so a `CDWAgent` install keeps its case. BAAM's Installed badge treats a row and an install as one extension when their ids match before or after a rename.

Every installer reads ONE `manifest.json` entry and then writes every entry, so each refuses a bundle in which a second entry is the same file on disk (the rules are in [The updater rules](#the-updater-rules), item 6): the Rust installers in `BrxtBundle::open` (`refuse_aliased_entries`, `extension_install/brxt.rs`), the desktop's file install and the updater in `safeExtractZip`. Each also reads the extracted `manifest.json` back and refuses the install when it is not the bytes it read (`BrxtBundle::extract_to`, `extractedFileMatchesEntry` in `ui/desktop/src/utils/safeZip.ts`).

Until the move runs, deleting gptr Agent from Settings (`brxt:uninstall`) removes the config entry and `extensions/gptr`, which does not exist yet, and leaves `extensions/gptragent` on disk. Nothing runs from it any more, so the move leaves it in place too; it can be deleted by hand. The extension manager's removal tool removes it with the entry.

The agent's marketplace install (the extension manager's install tool) refuses, before anything is written, a downloaded bundle whose manifest name is not one of the ids of the extension name its registry row states (`marketplace_install_transaction` in `agents/extension_manager_extension.rs`). Only a row that states `extension_name` is checked: without one, the name is the registry id by fallback, which is not the extension's name (SPOKEAgent's row said `spokeagent-0.4.1` and installed as `spokeagent`). Two installers do not make this check: the desktop's BAAM install, which shows the bundle's own name and privacy before the person confirms it, and `biorouter extension configure` finishing a parked marketplace install, which re-reads the bundle from the address the parked install recorded.

## The updater rules

The desktop app checks installed extensions for updates in the background. Until R it asked `releases/latest` of the repository in the installed manifest, took the first asset whose name ended in `.brxt`, and extracted it over the installed folder with no check of the asset's name, the bundle's manifest or a digest.

That rule breaks under a rename. GitHub lists release assets in case-insensitive name order, not upload order, so `gptr.brxt` and `gptr-X.Y.Z.brxt` sort before `gptragent.brxt` (observed on Broccolito/gptr's releases, not tested by uploading). A `gptr`-named bundle has a `gptr` script and no `gptragent` script (as gptr pull request 132 builds it), and an install whose config runs `gptragent` stops starting when that bundle is extracted over it. So, for as long as older desktop builds matter, `releases/latest` of Broccolito/gptr keeps only `gptragent`-named bundles, and the `gptr`-named bundle ships only on a prerelease tag that is never latest, `bridge-vX.Y.Z`.

From R, the updater (`ui/desktop/src/utils/extensionUpdater.ts`, which asks `extensionReleaseSource.ts` where a release comes from):

1. **Skips a folder named with an old id.** The daemon moves it first.
2. **Follows BAAM for a listed extension.** When exactly one registry row names one of the extension's ids (the live registry, its last good copy, or the copy bundled with the app), the version is the row's version and the bundle is the row's `download` URL (`https://github.com/<owner>/<repo>/releases/download/<tag>/<filename>`, with the row's `filename`). The digest is that asset's `digest` from GitHub's release API for that tag, because `registry.json` v2 has no digest field. A never-latest prerelease is reachable this way, which the turn-on needs. Two matching rows: no update, and a log line.
3. **Takes only `<installed id>.brxt` otherwise.** An extension BAAM does not list updates from `releases/latest` of its manifest's repository, and only from the one asset whose name is exactly its id plus `.brxt`.
4. **Refuses a bundle without a verified digest.** A release asset without a `sha256:` digest is not applied, and a bundle whose bytes do not hash to it is refused before the archive is opened.
5. **Refuses a bundle that is not the installed extension.** The bundle's manifest name must be one of the installed extension's ids (the installed name or its rename target), its entry point must equal the installed one, and its version must equal the release's.
6. **Extracts safely and can undo.** The manifest check reads ONE entry, `manifest.json`, and the extraction writes every entry, so a second entry that is the same file on disk would install a manifest nobody checked; that manifest then steers every later check (its name picks the registry row, its entry point is what the next bundle is held to, its repository is where an unlisted extension's releases come from). Three layers stop it, because no rule over names knows every disk:
   - **Before the install is touched**, the whole archive is checked (`assertDistinctZipEntries`). An entry with an absolute path, `..` or NUL in its name is refused. So are two entries that differ only by a `.` segment (`./manifest.json`, on every disk), by case under Unicode rules (`MANIFEST.JSON`, and `manifeſt.json`, whose long s a case-insensitive APFS reads as `s`), by Unicode form, or by trailing dots and spaces (which Windows drops), and a name with a colon (an NTFS stream: `manifest.json::$DATA` is `manifest.json`) or the shape of an 8.3 short name (`MANIFE~1.JSO`). The fold is wider than any one disk's, which only refuses more bundles.
   - **While extracting**, an entry that would land on a file an earlier entry of the same archive wrote (through an existing hard link or link inside the folder, or a disk's own folding) is refused before it overwrites it. `safeExtractZip` also refuses an existing symlinked component that leaves the folder.
   - **After extracting**, the `manifest.json` on disk must be the bytes that were checked; otherwise the update is refused.

   A refused or failed update restores the backup taken before extraction.
7. **Reads at most 256 MB.** A bundle is held in memory whole so its digest can be checked before anything is written, so the download is refused past that size, by the size the server states and by the bytes received.
8. **Reports a refused release once per run.** A bundle refused for what it is (its manifest or its entries) is refused the same way at every check, so it is reported once in a run of the app and skipped silently after that. A new release has a new digest and is judged again. A digest mismatch or a network failure is not remembered, because a retry can pass.

**Whose word the digest is.** The digest comes from GitHub's release API for the same repository the bundle is downloaded from, so it proves the download is the file GitHub published, and nothing more: a party who can change the release can change the asset and its digest together. What decides which release is installed is the registry row (for a listed extension) or the manifest's repository (for any other), and the manifest checks above. The catalog's own reviewed `release.sha256` is not served to clients, because `registry.json` v2 cannot carry it; a registry v3 read only by new clients could carry it and make the catalog the trust root.

**An install that stops updating.** An install of gptr 1.3.x from Broccolito/gptr (manifest name `gptragent`, repository Broccolito/gptr) now follows BAAM's `gptragent` row, which points at the archived GptrAgent v0.1.2. Version 0.1.2 is not newer than 1.3.x, so the updater reports it up to date until the row moves. Before R it updated from `releases/latest` of Broccolito/gptr. Moving the row to Broccolito/gptr is an ordinary catalog release (see [Release a new version of an extension](baam-catalog.md#release-a-new-version-of-an-extension)), best made right after R.

## Why registry v2 cannot carry a minimum app version

The natural way to move BAAM to `gptr` safely would be a row that says which Biorouter version can install it. `registry.json` v2 cannot say that:

- Every daemon since v1.90.0 parses it with `deny_unknown_fields` (`RawRegistry` and `RawExtension` in [`crates/biorouter/src/marketplace.rs`](../../crates/biorouter/src/marketplace.rs)) and falls back to a stale copy when it meets an added field, so a new field breaks the marketplace for every older client.
- `validate_metadata` refuses any `version` other than 2, so a v3 document at the same address is refused by every older client.
- No existing field can hold it. `version` is the extension's own version, the asset URL may carry no query and no fragment (`validate_https_url`), and its last segment must equal `filename`.

Old clients read the live registry, so the moment it names `gptr.brxt`, a client older than R can install the `gptr`-named bundle while knowing nothing of the rename: it shows a `gptragent` install as not installed, and its updater takes the first `.brxt` of `releases/latest`, a `gptragent`-named bundle, over a `gptr` install whose config runs a `gptr` script. So BAAM keeps pointing at `gptragent.brxt` (the row's current release) until the owner judges clients older than R rare. A registry v3 at a new address, read only by new clients, could carry a minimum version and a digest later.

## The gptr turn-on pull request

What R ships in the catalog is groundwork: the `gptragent` row in `landing/catalog/extensions.json` records `"renamed_locally_to": "gptr"` and `"gptr"` in its aliases, and `check-catalog.mjs` holds the two to the map. `registry.json`, its desktop fallback and `landing/baam.html` do not change, and there is no `gptr` row: the catalog's one-name rule (D2) reads `gptragent`, "gptr Agent" and `gptr` as one name, so a `gptr` row cannot stand beside an active `gptragent` row, and a catalog status has no "pending" state.

A later pull request turns BAAM to `gptr`. Its preconditions, all four:

1. R has shipped.
2. The gptr session has published `bridge-vX.Y.Z` on Broccolito/gptr, a prerelease that is never latest, holding `gptr.brxt`, `gptr-X.Y.Z.brxt` and `SHA256SUMS`, and has sent the tag and the SHA-256 of `gptr.brxt` (its line in `SHA256SUMS`, equal to the asset's GitHub digest).
3. The owner judges clients older than R rare.
4. The entry point prerequisite below has landed in a released Biorouter.

It changes, in one pull request:

| File | Change |
| --- | --- |
| `landing/catalog/extensions.json` | A new active row `gptr`: `manifest_name: "gptr"`, repository `Broccolito/gptr`, `aliases: ["gptragent"]`, version X.Y.Z with a history entry, `release: {tag: "bridge-vX.Y.Z", asset: "gptr.brxt", sha256: "<the sent sha256>"}`, and the job `r.live-session` and the bundled skill `gptr-live-r` moved from `gptragent`. The check's tag rule wants `vX.Y.Z`, so the row states why its tag differs in `release.legacy_tag`, or the same pull request teaches the check a named exception. The `gptragent` row becomes `superseded_by:gptr` and keeps `renamed_locally_to` and its `gptr` alias (the rename rule holds a superseded row too); its display name becomes `GptrAgent`, its repository's name, so that the new card can say "Replaces GptrAgent" as D4 requires when both rows are otherwise "gptr Agent". |
| `landing/baam.html` | The card's `data-extension-name="gptr"`, the `bridge-vX.Y.Z` download link and version, "Replaces GptrAgent" in its description, and one Moved and retired line for `gptragent`. |
| `landing/catalog/jobs.json` | The owner of `r.live-session` becomes `gptr`. |
| `landing/baam-details.js` | The `gptragent` key becomes `gptr`, with the `gptr__` tool names. |
| `landing/scripts/baam-privacy-facet.test.mjs` | The private cards it expects become `cdwagent`, `gptr`, `ucsfomopagent`. |
| Generated: `landing/registry.json`, `ui/desktop/src/components/baam/registry.fallback.json`, `crates/biorouter/src/privacy/registry_private.rs` | Run `node landing/scripts/build-registry.mjs`. The registry then holds `id: "gptr"` and `extension_name: "gptr"`. The private set and `EXPECTED_PRIVATE_EXTENSIONS` do not change: the generator now emits `gptragent` as the rename partner of the private `gptr` card. |

The entry point prerequisite: an install migrated by R still runs the `gptragent`-named bundle it was installed from, whose entry point is `gptragent`, and the updater refuses a bundle that changes the entry point (rule 5), so it would refuse `gptr.brxt`. One of two changes has to land in a Biorouter release before the turn-on: either the `gptr`-named bundle keeps a `gptragent` script and the updater accepts a bundle that still provides the installed entry point as a script, or the daemon rewrites the config's entry point when an update changes it. Until one lands, migrated installs keep updating only from `gptragent`-named releases.

## Tests

```bash
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter --lib config::extension_renames
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter --test extension_rename_migration
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter --lib privacy::
node --test landing/scripts/check-catalog.test.mjs
node --test landing/scripts/build-registry.test.mjs
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter --lib -- extension_rename extension_manager_extension extension_install config::permission
cd ui/desktop && npx vitest run src/utils/extensionRenames.test.ts src/utils/extensionReleaseSource.test.ts src/utils/extensionUpdater.test.ts src/utils/safeZip.test.ts src/utils/safeZip.symlinkBoundary.test.ts
```

## Related documentation

- [The BAAM catalog](baam-catalog.md): the catalog rules a rename works within, and the short procedure for the next one
- [Installing an extension, and where its credentials go](installing-an-extension.md): the install surfaces and the updater
- [Privacy tiers](../security/privacy-tiers.md): the compiled private set and why a renamed extension keeps both ids in it
- [Extensions and skills](README.md): the folder index
