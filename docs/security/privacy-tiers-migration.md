# Privacy tiers — what happens to your existing chats

> **What this is.** What the privacy-tiers upgrade does to the chats you already have: which
> ones are marked private, how that decision is made, what it cannot know, and what to do about
> a chat it got wrong.
> **Status:** Current — describes the one-time migration that ships with privacy tiers
> ([issue #56](https://github.com/BaranziniLab/biorouter/issues/56), §15).
> **Audience:** anyone upgrading an existing Biorouter install, and support answering "why is
> this chat suddenly refusing my model?"

## The release note

> *"Chats from before this version are marked by the model they were last using. If an older
> chat contains work you want kept private, switch it to a private model — it will be marked
> private from its next turn on."*

## What the upgrade does

The first launch after upgrading adds three columns to the session database, creates the
declassification ledger, and then runs a **one-time backfill**: every chat whose last bound
model was a private-tier provider — Versa (Azure), Versa (Bedrock), Llama Server or Ollama —
is marked **private**, with a provenance of `backfill:<provider>`.

Everything else is left **public**. That includes chats with no recorded model at all, which on
a real machine is the largest group of the three.

Before enforcement starts affecting your chats, Biorouter shows a one-screen notice with the
**actual numbers from your own database** — how many chats were marked, broken down by the
model that marked them, and how many record no model at all. Those numbers are computed at
that moment; nothing in the product quotes a figure from a design document.

The notice appears **once**, on the first launch after the backfill has marked something you
can see, and it is dismissible. If you never upgraded across this version — a fresh install —
you will never see it, however many private chats you go on to accumulate: everything it says
is about a change made to chats that already existed.

It also names one thing that did **not** change, because nothing else will ever tell you: any
extension you have enabled that is wired to clinical data and is **not** marked private. Those
stay callable by any model, including commercial models hosted outside UCSF, exactly as they
were before the upgrade. See [Privacy tiers §13.5](privacy-tiers.md).

## Why some chats are marked private and others are not

The backfill reads one field: the model a chat was **last** bound to. It does not read a single
message.

That has a consequence worth stating plainly, because you will meet it:

- A chat that ran on Versa and was later switched to a commercial model is marked **public**,
  even though its transcript contains private-model work.
- A chat that ran on a commercial model and was switched to Ollama for one turn is marked
  **private**, even though nothing sensitive is in it.

There is no transcript scan and there will not be one. Scanning message bodies to decide a
privacy tier would mean reading every conversation on the machine to protect the few, and it
would still be a guess.

## Why the unknown cases are left public

A chat with no recorded model could be anything. The migration marks it public.

This is deliberate, and it is the least-bad option rather than a comfortable one. The
alternative — treat "unknown, and it has messages" as private — was rejected because of what it
does to someone who has never used a private model at all: a large slice of their history would
come back private on first launch and refuse the model they normally work with, and the only
way out is declassification, one chat at a time, irreversibly. A wrong guess in that direction
costs the user their history; a wrong guess in this direction leaves a chat exactly as exposed
as it was before the upgrade.

## Fixing a chat the migration got wrong

**A chat that should be private but is not:** open it and switch it to a private model. From
its next turn on it is marked private, and it stays that way.

**A chat that is private and should not be:** declassify it, from the chat's own privacy
control (or `biorouter session declassify <id>` for a chat History does not list). This is
one-way and it is recorded in an append-only ledger. Chats marked by the backfill get the
stronger confirmation, because a `backfill:` tier is an **inference Biorouter made from the
last-used model**, not something you told it — unlike a chat marked because a turn actually ran
against a private endpoint.

## Knowledge bases

Knowledge bases that already exist start **public**, whatever fed them, for the same
fail-open reason as the sessions above: the tree keeps no record of which chat wrote which
page. If you know a base holds private material, mark it private yourself from the Knowledge
view — you do not have to wait for the next private ingest to raise it.

## Running it twice

The backfill runs **once**, from schema migration **19**, and never again. This matters:
declassifying a chat deliberately leaves its bound model alone (a public chat is allowed to run
a private model), so a backfill that re-ran on every launch would silently re-mark every chat
you had just declassified. That is the one user-only action in the whole feature, and nothing
in the product may undo it on your behalf.

The columns and the ledger arrive one migration earlier, at **18**, and are also repaired on
every startup if they are missing. That split is deliberate and it is not cosmetic. Adding new
work to a migration number that some installs have already passed means those installs skip it
silently and forever — which is what happened here during development: the backfill was
originally written into 18, and every machine that had opened a development build was already
at 18 and would never have run it. A schema number is consumed the moment anyone applies it.
If you are on a development build that already reported 18, migration 19 is what backfills you.

For support: the migration logs its four counts at `info` on the run that performs it —
`backfilled_private`, `backfilled_public_named`, `backfilled_unknown_provider`,
`backfilled_empty` — alongside the three visible counts the notice showed. A `-1` in any of
them means that count query failed; the backfill itself had already committed by then, and it
is deliberately not allowed to fail the migration, because a report that aborted `apply_migration`
would leave the database backfilled and still numbered below the arm.

## If you turned privacy tiers off by hand

The master switch used to be a `config.yaml` key, `BIOROUTER_PRIVACY_TIERS`. It is now a record
of its own, `privacy-tiers.json`, in the same directory — because a key in `config.yaml` was
writable by anything that could write files, including the agent, which made the one control the
agent is subject to something the agent could switch off and wait for a restart.

**You do not have to do anything.** The first launch after upgrading reads your key once, carries
your answer into the new record, and removes the key. If you had it set to `off`, it stays off.

From then on the key is **ignored**: putting it back into `config.yaml` does nothing, whatever it
says. Change the switch in **Settings → Privacy**, which is the only thing that writes the record.
If you want to check the state from outside the app, read `privacy-tiers.json` — `{"enabled":
false}` means enforcement is off.

**While privacy tiers are off, the app says so.** A note stands above every composer — Home's
and every chat's — and Settings → Privacy repeats it, with where the switch is recorded and how
it got to off: turned off in Settings → Privacy, carried over from your old `config.yaml`, or —
when the app recorded no such change — *turned off outside the app*, meaning
`privacy-tiers.json` was edited directly. The note has no dismiss button; turning privacy tiers
back on is what clears it. The daemon also logs one warning at every start-up while they are
off.

If you turned privacy tiers off in Settings → Privacy with a version from before that note existed,
your record carries no trace of which door wrote it, so the note reads *turned off outside the app*
and says an older version could be the reason. To have it describe your change accurately, turn the
tiers on and then off again in Settings → Privacy.

## Rolling back

Downgrading leaves the columns in place and ignored, and the ledger inert. Nothing is moved,
re-indexed or rewritten. Rolling forward again re-runs nothing, because the migration is
versioned. The one-way door is the backfill itself.

A downgrade *does* stop the older build seeing your master switch: it looks for the retired key,
which the upgrade removed, and so comes up with privacy tiers **on** — the fail-safe direction,
and the same answer it gives a fresh install. Setting the key again turns it back off on the old
build, and the newer build will go on ignoring it.

## Related documentation

- [Privacy tiers](privacy-tiers.md) — the design this migration serves: how models, sessions,
  extensions and knowledge bases acquire a tier, and the enforcement gates. §15 covers the
  migration and §16 the measured cost.
- [Privacy tiers — implementation plan](privacy-tiers-execution-plan.md) — the execution plan,
  including this task's own gates.
- [Data privacy and patient data](data-privacy-and-phi.md) — which providers are acceptable for
  PHI in the first place.
- [Security](README.md) — the rest of the security documentation.
