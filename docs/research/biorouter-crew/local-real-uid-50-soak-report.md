# Local real-UID 50-profile soak

This report records the second disposable Linux workload run completed on
2026-09-22. It used the current Linux artifact in the standalone broker only;
the three live GUI clients and the shared three-user SSH fixture were not under
this workload.

## Provenance and portability

- Binary: `/usr/local/bin/biorouter-crew`, SHA-256
  `7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759`.
- Source fingerprint supplied with the binary: broker source `995dbf1d9b377b57060010366fa899d1e6c3754a97311eeeb3e5faab2679ada4`.
- The artifact ran in Debian GNU/Linux 13 (trixie), from cached image
  `rust:latest` digest `sha256:bf5a9aa29062a6cb03c49bd59a46eb55e3cc770caf598a221a7866e500be3082`, under the synthetic rootless owner UID
  11900 and participant UIDs 12001–12050. A Bullseye fixture refused to start
  it because the aarch64 artifact imports GLIBC_2.39; this is Debian trixie
  evidence, not a Debian 11 portability result.
- Durable evidence was copied before cleanup to
  `/private/tmp/biorouter-crew-soak-50-rerun-luna/` (metrics, runtime,
  journal, marker data, result, and replay oracle).

## Results

The run started at `2026-09-22T13:42:00.462195+00:00` and ended at
`2026-09-22T14:12:31.965220+00:00` (the requested 1,800-second workload).
All 50 participants produced terminal summaries. Totals were 1,500 sent,
1,500 acknowledged, and 1,500 read back, with 1,500 globally unique posted
IDs and 1,500 globally unique readback IDs. The 1,500 posted and readback body
hashes matched exactly; errors and disconnects were zero.

Latency over the per-message records was p50 172.94 ms, p95 712.79 ms, and
p99 903.92 ms. Broker maxima collected by the harness were RSS 1,262,800 KiB,
CPU 8,661 ticks, and 106 open file descriptors; journal size peaked at
2,233,601 bytes.

The retained broker was stopped and restarted with the same state. Owner
`auth.bootstrap` is correctly rejected after enrollment, so replay used
enrolled synthetic participant UID 12001 under its real kernel UID. Paginated
`messages.history` (`latest=false`) replayed all 1,500 messages after restart:
`expected=1500`, `replayed=1500`, `missing=[]`, `mismatched=[]`. The current
run's container copy predates per-participant timestamp fields, so the report
proves the top-level 1,800-second interval but does not claim an independent
minimum duration for each participant.

This was standalone broker traffic and does not constitute integrated GUI
under-load evidence, real Duo/TOTP evidence, or Debian 11 portability evidence.

## Initial allocator diagnostic deferral

An initial planned ≤120-second allocator comparison using the same 7311 artifact and
`MALLOC_ARENA_MAX=2` was not launched: the host had 195 GiB free, below the
200 GiB safety floor for starting another fixture. That attempt produced no
allocator evidence. After capacity recovered, the separate bounded comparison
below ran. The harness also rejects duplicate posted or readback IDs across
participants before reporting a soak pass.

## Bounded allocator diagnostic

A separate diagnostic used two fresh equivalent Debian GNU/Linux 13 (trixie)
cached-image containers (the `rust:latest` digest above), the unchanged 7311
artifact, 50 synthetic UIDs, and a 60-second workload at two-second
intervals. Both used Debian GLIBC `2.41-12+deb13u3`.
The default run used `MALLOC_ARENA_MAX=0` (glibc default); the comparison used
`MALLOC_ARENA_MAX=2`. Each run stopped its exact rootless broker and the
containers were removed afterward. The live three-user fixture was untouched.

| Setting | Sent/acked | p50/p95/p99 ms | Harness RSS max | Sample RSS max | 5-second idle RSS |
| --- | ---: | ---: | ---: | ---: | ---: |
| glibc default | 1,055/1,055 | 299.61/1,532.62/1,832.16 | 861,224 KiB | 868,900 KiB | 91,160 KiB |
| `MALLOC_ARENA_MAX=2` | 1,050/1,050 | 332.82/1,237.56/1,605.60 | 57,156 KiB | 57,428 KiB | 45,772 KiB |

The two totals differ slightly (1,055 versus 1,050) because each 60-second
run used the same participant count and interval design but independent timing;
they are not byte-identical workloads.

Both runs reported zero errors and disconnects, and global posted/readback IDs
were unique. PSS sampling was attempted through `/proc/<pid>/smaps_rollup` but
was denied by the container proc policy, so no PSS value is claimed. This is a
controlled observation that allocator configuration changed measured RSS in
this short fixture; it does not establish a leak, explain all production RSS,
or justify changing product defaults.

Durable result and sample logs are in
`/private/tmp/biorouter-crew-alloc-diagnostic-20260922/`.
