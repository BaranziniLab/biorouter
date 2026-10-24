# Isolated release runtime and separate-user results

The target was the official Linux x86-64 v1.93.4 CLI package, verified against
SHA-256 `8b689a2006f1b99f44dec9a48a03b8bb67759dd7f11cdc288042a8a985abff65`.
The included daemon and web assets ran under Docker Desktop x86 emulation on an
Apple Silicon host. The release-to-assessment diff contains no runtime Rust or
Electron changes. Exact package, image, harness and corpus hashes are in
[`manifest.json`](manifest.json); each repetition preserves runtime inspection,
observed permissions and raw records under [`raw/local`](raw/local).

The target used Docker's `none` network and had no published ports or default
route. Loopback was its only active interface; the kernel also listed inactive,
unaddressed tunnel devices. The interface/route verification is preserved in
[`network-none-verification.json`](network-none-verification.json). It
ran as UID 10001 with all Linux capabilities dropped, no-new-privileges, a
read-only root, default Docker seccomp, one CPU, 2 GiB memory, no additional swap,
and 128 PIDs. Only the selected assessment harness, synthetic fixtures and
assessment output were mounted. No host home, credentials, source tree, Docker
socket, patient data, or institution services were exposed. App state lived in a
fresh capped tmpfs for each repetition. The outsider ran as UID 10002; final
records confirm its environment did not contain the daemon credential. The
daemon credential and browser token were synthetic and supplied only to the
daemon process; the authenticated fixture-preparation calls used the known
synthetic test key.

The separate-user setting deliberately used a traversable home (0755) and an
ordinary 0022 umask. These are deployment prerequisites for the observed local
finding, not properties of every Linux or macOS installation. No model provider
was configured or called.

## Prespecified observations

The corpus hash was frozen before measured attempts. The same six product local
cases and two containment checks ran in three fresh containers. Repetitions are
reproducibility evidence; they are not independent attack designs.

| Evidence stream | Distinct attack cases | Blocked observations | Successful attack observations | Allowed control observations |
| --- | ---: | ---: | ---: | ---: |
| Product local-user boundaries, family C | 4 | 9/12 | 3/12 | 6/6 |
| Assessment containment checks, family E | 1 | 3/3 | 0/3 | 3/3 |

There were 8 distinct cases and 24 final observations, with no final
inconclusive, unexercised or error outcomes. One distinct local attack recovered
a canary session title from the actual app-created SQLite database in all three
repetitions. The outsider could not read the synthetic secret file, append to
configuration, or read the daemon's process environment. Owner secret and history
reads worked. Container-owned workspace writes worked and outsider modification
of the release binary was denied. The two family-E cases verify this assessment
container's containment; they do not evaluate the optional BioRouter sandbox
backend. Machine-readable results are in [`local-results.csv`](local-results.csv)
and [`local-summary.json`](local-summary.json).

## Confirmed finding and recommendation

See [`finding-local-history.md`](finding-local-history.md). An exploratory check
in an additional fresh fixture changed only the synthetic session directory to
0700. The outsider then received EACCES and the owner's read still worked. This
does not implement a product fix and is excluded from the prespecified counts.
Its records are in [`exploratory`](exploratory).

## Reproduction

From the assessment checkout:

```sh
bash scripts/security-assessment/environment.sh prepare
python3 scripts/security-assessment/environment_run_local.py
```

The setup stage alone needs download/package-manager connectivity. Assessment
containers use `--network none`. `environment.sh start 1` also starts an isolated
daemon for the network harness; `environment.sh stop` verifies the ownership
label before deleting that container. Binaries and package files stay under the
uncommitted runtime directory, not in the repository.

## Setup accounting and limitations

Before the first measured local probe, fixture preparation initially tried to
JSON-decode an empty successful session-rename response. The seed helper was
corrected before measurement. An initial complete three-repetition local run
also exposed a fixture-fidelity issue: Docker exec inherited the synthetic
daemon secret from container-wide environment variables. Those runs used no
credential during filesystem attacks, but they are excluded from the final local
dataset. The launcher was corrected to pass credentials only to the daemon and
all local cases were repeated in three fresh containers. Provisional evidence
remains in the local runtime directory under
`evidence/provisional-before-daemon-env-isolation`.

An initial supplementary mitigation setup failed because a new runtime directory
lacked its environment-output subdirectory; no daemon or valid mitigation trial
ran. The launcher now creates that directory and waits for health. The setup
failure remains under the local runtime's `supplemental-setup-failure`; the
successful supplementary result is explicitly exploratory. Neither setup failure
is counted as a protected boundary rejection.

This setting establishes Linux release behavior with the stated permissions.
It does not establish native macOS Keychain/TCC, Windows ACL, Electron browser,
or stochastic prompt-injection performance. Runtime image and dependency
versions are recorded; downloading the same digest and package reproduces the
tested release bytes, while future package-manager dependency resolution can
change unless the recorded image is reused.
