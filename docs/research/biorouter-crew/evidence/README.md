# Crew screenshot evidence

These are synthetic-data captures from isolated Electron development profiles.
They were reviewed for readable controls and visible credentials before inclusion.
A screenshot proves the visible state at capture time; it does not establish a
model tool result, a completed native Save, or acceptance on a later binary.
See the [app evidence](../crew-ui-acceptance-report.md) and
[matched backend traces](../local-provider-boundary-report.md) for those distinctions.

| Capture | What to inspect |
|---|---|
| [Restricted channel members](fresh-control-members.png) | Three enrolled participants in the clean restricted channel |
| [Remote CSV read](fresh-control-result.png) | Rendered read result, separately matched to a typed backend response |
| [Personal conversation grant](bob-personal-crew-grant.png) | Explicit destination and posting consent; the later model request failed |
| [Personal greeting](bob-personal-greeting.png) | Existing personal conversation before the Crew grant |
| [Standalone slash navigation](bob-slash-existing2.png) | Existing session carried into Crew |
| [Image preview](carol-blocks-preview.png) | Synthetic image preview; a completed named Save was not established |
| [Opaque attachment](carol-opaque-uploaded.png) | Downloadable binary file card without an image preview |
| [PAM authentication](carol-fresh-pam-after-password.png) | Authentication completion, separate from full broker connectivity |
| [Authentication closed](carol-pam-cancelled.png) | Return to the disconnected authentication state |
| [Failed processing correction](carol-processing-correction.png) | Historical failed agent attempt, retained as a failure |
| [Failed selected-channel retrieval](alice-cross-channel.png) | Historical model result rejected for lack of matching retrieval |

Additional reviewed captures preserve the chronological states referenced by the
reports. Two byte-identical historical copies remain local and are not included
in this directory's committed image set: `carol-personal-crew-after.png` and
`final-9413-reconnected.png`; their canonical image is `carol-agent-final.png`.
No screenshot in this collection establishes production HIPAA compliance.
