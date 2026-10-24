# Workspace path resolution and file creation


**Status:** two dangling-link escape paths reproduced across two sandbox file APIs in six baseline observations. The unmerged, unreleased assessment branch contains fixes plus repeated sandbox and public-editor regressions. The scope is path validation; concurrent path replacement remains unassessed. Tracked by [issue #446](https://github.com/BaranziniLab/biorouter/issues/446).

## Baseline observation

The common resolver canonicalized an existing ancestor and appended unresolved components. A dangling symlink appeared missing to the initial existence check. A subsequent write followed the link and created a file outside a synthetic workspace. Both `LocalProcessSandbox::write` and the host-side `DockerSandbox::write` API exhibited the behavior. The baseline component source corresponded to the v1.93.4 runtime source; this was not an in-container command escape or evidence of outside-workspace shell execution.

The eight-case file-API set was repeated three times: six dangling-link writes escaped, fifteen other negative observations were denied, and three ordinary controls were allowed.

## Source changes and post-change checks

The assessment branch rejects unresolved symlink components. Source review extended the work to normal editor resolution, diff-selected child paths, supported overlap prefixes, directory creation before patch application, and explicitly user-created remembered directory grants. Ambiguous paths no longer inherit those grants. Scoped grants are enabled as a tier by default, but grant nothing until a user explicitly creates a matching grant.

The sandbox replays repeated the same eight-case set three times. All twenty-one negative observations were denied, all three controls were allowed, and there were no outside writes.

The public-editor regression used eleven checks per run: seven unsafe paths and four controls. Across three runs, it recorded twenty-one denials and twelve allowed controls, with no outside files or directories created. Controls exercised an ordinary direct write, nested diff-created file, supported overlap-prefixed nested file, and an existing internal symlink through direct editor write. These results do not establish internal-symlink behavior for every backend or operation. The extra editor paths were identified by source review and checked after remediation; they were not all separately reproduced against the original release.

The scope regression rejected unresolved link paths while preserving a normal new-file directory grant. It did not remove explicit human permission choices or create new automatic grants.

## Recommendations preserving normal use

Keep ordinary nested file creation, supported internal links, patch application, and explicit scoped approvals working with clear errors on rejected paths. The regression controls document those exercised workflows.

For a stronger long-term boundary, use directory-handle-relative opens and directory creation with protections against link traversal and concurrent path replacement. Preserve the existing editor/file APIs so researchers do not need extra steps for ordinary work. Keep ambiguous paths subject to a fresh decision rather than inheriting a directory grant.

The current checks are not a race-resistant open design. Workspace file-API tests do not establish OS process containment; LocalProcessSandbox commands run as the host account. Full Docker command execution was not assessed.

## Evidence locations

- `tool-boundary/raw/file-api-repeat-{1,2,3}.log`: baseline sandbox observations.
- `tool-boundary/raw/post-change-file-api-repeat-{1,2,3}.log` and the summary JSON: repeated sandbox regression observations.
- `tool-boundary/raw/post-change-editor-repeat-{1,2,3}.log`: repeated public-editor observations.
- `tool-boundary/raw/post-change-scope-regressions.log`: scope/configuration checks.
- The source patch records changes to sandbox, editor, diff, and scoped-permission resolution.
