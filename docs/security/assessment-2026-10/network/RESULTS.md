# Observed network/API results

The 55-case protocol corpus was repeated against three fresh daemon/state
instances of the official v1.93.4 Linux amd64 package. All three repetitions
produced identical case-level outcomes.

| Family | Unique attack cases | Blocked attack observations | Allowed control observations | Unexercised control observations |
| --- | ---: | ---: | ---: | ---: |
| A: API authentication | 20 | 60/60 | 27/27 | 0 |
| B: browser/WebSocket protocol boundaries | 20 | 60/60 | 15/15 | 3 |
| Total | 40 | 120/120 | 42/42 | 3 |

There were no observed successful attacks, blocked exercised controls, errors, or
inconclusive case outcomes in this corpus. The 165 case observations contain 255
individual protocol requests; grouped request sequences remain one case design.
These counts describe the finite tested corpus and do not estimate a deployment
compromise probability.

Authentication attacks did not disclose the seeded configuration marker or alter
its persisted value. Alternate API credential channels and route shapes did not
produce protected responses. A separate loopback peer received twenty `401`
failure responses followed by five `429` responses despite changing the supplied
forwarding header; its valid credential still worked immediately afterwards.

Workspace sockets rejected missing or incorrect secrets and hostile, opaque,
undeclared file, and alternate-loopback-port origins. The valid same-origin and
non-browser controls upgraded successfully. These correct-secret origin probes
isolate the server's origin rule; they do not demonstrate that an outsider can
obtain the synthetic provisioned credential.

Valid browser-token exchange and cookie navigation worked. Cookie-bearing script
and iframe fetches, insecure nonloopback addresses, and cookie-only REST requests
were denied in the tested protocol forms. The shell carried the expected
no-store, framing, opener, and content-security protections. These are
handcrafted protocol observations, without an actual browser engine.

B-U01 remains unexercised in every repetition because no live coding-agent turn
issued and revoked a real bridge grant. B-19 exercised only absent app IDs, so
existing-versus-absent enumeration was not evaluated. Deliberately weak/manual
daemon credentials, disabled browser tokens, actual reverse-proxy deployments,
native macOS/Electron behavior, and stochastic model attacks were not evaluated
by this corpus.

No new network defect is supported by these baseline results. Raw records,
response evidence, the frozen corpus, and machine-readable totals accompany this
file; see `README.md` for reproduction and further limits.
