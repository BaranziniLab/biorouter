# Privacy and security

> **What this is.** The Crew manual page on privacy and security: which AI models may read your work, what other people can see, and what full agent access and open workspaces risk.
> **Status:** Current. Checked against the Crew code on 2026-10-16.
> **Audience:** Lab members who use Crew in the Biorouter desktop app, and workspace hosts.

Privacy in Crew decides which AI models may read your lab's messages and files, never which people see them. Only people who were let in see a workspace, by the host or in an open workspace by any member, and only a channel's members see the channel. The host is the member who runs the workspace on the server. A word in braces, such as {workspace}, stands for a name Crew fills in.

## What Private and Public allow

A private model runs on your computer or at your institution. Biorouter lists it on the **Local** or **Institutional** tab of its model provider settings and marks it "Private", for example "Private · UCSF". A public model is a commercial service, on the **Public** tab. Local models, from Llama Server or Ollama, are approved for every institution.

In each workspace, your connection is "Private" or "Public". You choose it, and this computer saves it. The workspace is "Private for everyone" or "Allows Public". Only the host changes it, and a new workspace starts "Private for everyone" with no institution. Your privacy is Public only when your connection is Public and the workspace allows it. A Public connection never changes what you can read.

Crew protects what an agent reads when your privacy is Private, when a channel it reads is Restricted or holds anything Restricted, when it uses your remote work folder, or when the chat already holds Restricted material.

| Model | Protected content | Other content |
|---|---|---|
| Public | Refused | Allowed, except the remote work folder |
| Local, or approved for the workspace's institution | Allowed | Allowed |
| Private, approved elsewhere or stating no institution | Refused | Allowed |

Protected content also needs a workspace institution that matches your connection's. In a Private workspace with no institution, people can chat and share files, but no agent can work.

In **Ask my agent**, a model that is not approved shows "Not approved for {institution}", and **Start my agent and allow posting here** stays unavailable. A public model on a Restricted channel is refused when you start. A chat connected with `/crew` keeps its model. See [Agents and chat access](agents-and-chat-access.md).

## Where each rule is enforced

The Biorouter background service on your computer and Crew on the server make every decision, even when **Privacy tiers** is off in Biorouter's settings. The Crew window decides nothing.

## Check a workspace's privacy

The privacy chip sits at the right end of the status row, under the workspace name. It reads "Private · {institution}", "Private" or "Public", and changes only after the workspace confirms a setting. "Checking privacy…" means Crew is still confirming.

Choose the chip to open the privacy panel, which shows both settings, the institution and the reason. When the workspace allows Public, it offers **Make my connection public…** or **Make private**. **Privacy…** opens the **Privacy** tab of "{workspace} settings", which the workspace menu also opens.

## Channel labels

Each channel has a "Restricted" or "Public-safe" badge. Public models may read a Public-safe channel only when your privacy is Public, and never a Restricted one. Neither label limits who sees the channel. A new team's `#general` is "Public-safe" only when the workspace allows Public.

### Messages marked Restricted

The server also marks a message or file "Restricted" when:

- It was posted from a Private connection, or while the workspace was Private for everyone.
- Its channel is "Restricted".
- It was made from Restricted material, such as an agent result that read a Restricted message.
- An agent posted it from a Private task, or from one using a remote work folder.

In a "Public-safe" channel, such a message shows a muted "Restricted" label and closes the whole channel to public models, though the badge still reads "Public-safe". It stays Restricted after the host allows Public.

Names are not private: a taken team or channel name reveals that it exists, and anyone who can sign in to the server sees the workspace name. Keep patient and sample IDs out of names.

## Change your connection's privacy

Crew does not show your choice to the host or other members. Your messages, uploads and agent tasks carry it to the server, where the host's server account can read it. To change it from a terminal, see [Privacy settings](command-line.md#privacy-settings). Going public there asks you to type the workspace name, as the desktop does; in a script, add `--confirm WORKSPACE`.

Saving a new privacy or institution ends chat access granted through this connection. If the workspace was connected, Crew reconnects at once and the chip shows the new setting.

### Make your connection Public

1. Choose the privacy chip, then **Make my connection public…**, offered when the workspace allows Public. The **Privacy** tab has the same button, and saving **Public** in **Connection settings…** opens the same dialog.
2. Type the workspace name in **Type {workspace} to confirm**, then choose **Make public**.

The chip reads "Public". Your unsent draft is cleared, and chat access granted through this connection ends. A connected chat says "Crew settings changed since access was granted. Grant access again from Crew."

### Make your connection Private

1. Choose the privacy chip, then **Make private**. The **Privacy** tab and **Connection settings…** also offer it.
2. If Crew asks for an institution, type it, for example `ucsf`, and choose **Make private**.

No confirmation appears, and the chip reads "Private". Without an institution, Crew refuses with "Choose this private SSH connection's institution before saving".

## Change the workspace's privacy (host only)

The host has these buttons on the **Privacy** tab of "{workspace} settings". No agent can use them.

- **Allow Public…** asks you to type the workspace name. Members may then make their own connection Public.
- **Make Private for everyone…** makes everyone Private again.
- **Set institution to {id}…**, then **Set {id} permanently**, sets the institution for good.

Each change ends all agent access, and every member must grant it again. Labels and Restricted messages stay as they are.

## Institution IDs

An institution ID is a short lowercase ID for your organization, such as `ucsf`: 1 to 64 letters, numbers, `-` or `_`, starting with a letter or number. Some screens show the name a model provider publishes instead, such as "UCSF".

Your invitation usually fills in your connection's institution, and you can change it in **Connection settings…**. The host sets the workspace's institution once, right after **Create workspace** or later on the **Privacy** tab. It never changes, so another institution needs a new workspace. The two must match. See [Hosting a workspace](hosting-a-workspace.md).

One computer uses one institution per server. Saved workspaces on one server also share one privacy: if any is Private, all are.

## Privacy refusals

When Crew refuses an agent or a setting for privacy, the message names the rule. [Refusals when you start or allow](agents-and-chat-access.md#refusals-when-you-start-or-allow) covers the refusals for agents and chats, and [Other messages](connections-and-troubleshooting.md#other-messages) covers the institution and privacy checks on your connection.

## Keys, fingerprints and codes

You send your code, such as `7QK2-M9XA-3JTP-WZ4D`, to the host to be let in. Your computer makes it, so the server cannot change it. The workspace fingerprint, such as `3F2A 9C1E 77B0 D4E1`, is never sent. To check an invitation, open **Check this invitation (optional)** when you join, and join only if your host reads the same fingerprint from Crew.

Your device key's private half never leaves your computer. **Keys and security…**, in the You menu at the bottom of the Crew sidebar, shows where your keys are stored and each device on your account.

### New device notice

When a device joins your account, the connection bar says "A new device was added to your account on {date}." Choose **Review**, and tell your host about any device you do not recognize.

### Use an encrypted vault

A vault keeps your device keys in a file locked by a passphrase, instead of the system keychain. Set it up before you host or join your first workspace on this computer, because keys already in the keychain do not move. A Linux computer with no keyring service, such as a server you reach only over SSH, needs the vault: Crew never keeps a device key in a plain file, so without one it cannot save a key and says "This computer has no keyring service Biorouter can use…". If Crew has already used them, setup fails with "Vault initialization was refused. Use a fresh Crew profile…".

1. Open **Keys and security…** from the You menu.
2. Choose **Use an encrypted vault instead**, then **Set up vault…**.
3. In "Initialize Crew encrypted vault", type a new passphrase.
4. In "Confirm Crew vault passphrase", type it again.

**Keys and security…** now reads "Stored in an encrypted vault" and offers **Lock**. When the vault is locked, the connection bar says "Your Crew vault is locked." Choose **Unlock** there or in **Keys and security…**, then type the passphrase.

> **Warning.** Nobody can recover a lost passphrase. If the vault files go missing, restore them from a backup. See [Keep device keys in an encrypted vault](command-line.md#keep-device-keys-in-an-encrypted-vault).

## Server identity

Crew reaches the server over SSH and connects only to a server whose key is already in your known hosts file, usually `~/.ssh/known_hosts`. It never accepts a new or changed key. See [Server identity checks](connections-and-troubleshooting.md#server-identity-checks).

## Files Crew will not share

Crew refuses to share credential files such as `.env`, `secrets.*`, private keys, cloud credentials and password stores. It judges each file by its name and by its content, reading the first and last 64 KB of the file, so a renamed key file is refused too. A credential in the middle of a file larger than 128 KB is beyond this check. It shares public keys such as `id_ed25519.pub`. It also refuses to save into a credential or settings folder. See [Messages and files](messages-and-files.md).

With **In a folder** access, your remote work folder cannot be your home folder or overlap a protected folder such as `~/.ssh` or `~/.aws`. Only a private model can use it. With full access the agent can reach those folders, so Crew's secret guard reads each command and file path the way the server would, and refuses one that names a credential file, and it withholds results that carry a key or token ([Full access to the server](#full-access-to-the-server)).

## Full access to the server

**As me on the server**, the full level of [agent access](agents-and-chat-access.md#what-your-agent-may-do-on-the-server), lets your agent run commands as your own server account. It is your choice, for your account only, and it is the default for a connection made by joining or hosting, because agents that compute on the server need it. Choose **In a folder** or **Off** in **Connection settings…** if you do not want it.

What it risks:

- **Your whole account.** The agent can read and change anything your account can, use the network, and start programs that keep running after its access ends. A misled agent could delete files, send data off the server, or replace your own copy of Crew there.
- **Other people's words.** Messages, files and the workspace context written by other members reach your agent. Crew tells it to treat them as information, never as instructions, but a crafted message can still try to steer it. Use a private model you trust, and give tasks that say what to touch.
- **Host accounts.** If you host the workspace from that account, your agent can read everything the workspace stores, including channels you are not in.

What it does not change:

- A public model never gets any server access, whatever the level.
- Your device keys stay on this computer, so the agent still cannot sign in to Crew as you, change memberships or privacy, or approve its own access.
- Each command is announced in the task's steps by its program and length, never its text, and the secret guard still refuses credential files and withholds credentials from results.
- Revoking a chat, or a task's end, stops new commands. A detached job keeps running until it ends or you stop it.

### Open workspaces trust every member

A workspace the host creates is open. In an open workspace every member can rename, archive and delete any team or channel they are in, add and remove people in them, join any of them, invite new people and let them in. Only the host removes someone from the workspace, changes its privacy or governance, renames it or deletes it.

- A new member is added to every team and every channel that is not archived, so they can read everything already posted in all of them, files included.
- Any member can delete a channel or a team for everyone, and nobody can bring it back.
- Adding people in an open workspace does not end agent access, so a task or chat that reads a channel goes on reading it after someone new joins.

The host can switch the workspace to managed at any time ([Choose who manages the workspace](hosting-a-workspace.md#choose-who-manages-the-workspace)), and back. Switching to managed also stops invitations made by members from admitting anyone.

## What other people can see

Other members see your name, `@username` and posts in channels you share, with their "Restricted" labels. Everyone in the workspace sees whether you are online, meaning your computer is connected to the workspace now or made a request in the last three minutes. Your agent posts as "{your name}'s agent", and the Source line Crew adds to each task result and each connected chat's post names the shared files it read. Members never see channels they are not in, your privacy, institution, devices or connected chats.

In Crew, the host sees only the channels the host is in, plus the members, people waiting to join, and a warning when a computer shows a different code. The host never sees your password, the verification codes you sign in with, or your keys, and cannot start, stop or approve your agent. Some workspace changes end every agent's access, yours included ([Why settings changes end access](agents-and-chat-access.md#why-settings-changes-end-access)).

> **Warning.** The workspace's data sits in files owned by the host's server account. That account, programs running under it, including the host's agent with full access, and the server's administrators can read everything, including Restricted messages and channels the host is not in.

A history file under the host's account records each change and who made it. Deleting a channel or a team removes it, its files and its messages from the workspace, but the history file keeps the old records, unreadable in Crew, until the host deletes the whole workspace. The host's account can edit that file. Crew has no screen for it. See [Deletion and retention](administration.md#deletion-and-retention).

### Other accounts on the server

People who can sign in to the lab server, members or not, can learn some things about a workspace without joining it:

- The workspace's name, from the name of its folder and from the host's `biorouter-crew` process.
- Who is connected, and when. While your computer is connected, a `biorouter-crew bridge` process runs under your username with the workspace's ID on its command line, so `ps` shows it to every account on a server that does not hide other people's processes.
- The workspace's name, privacy mode, institution and key fingerprint. Any account on the server can ask the workspace's socket for them, because members' own accounts reach it the same way.

They cannot read messages, files, member lists or channel names this way: every other request needs a member's key. An administrator can hide other accounts' processes on a shared server, as [Administration](administration.md#what-other-accounts-can-see) describes. Keep patient, sample and study identifiers out of workspace, team and channel names.

### Other accounts on your computer

The background service on your computer listens on a network port that only programs on the same computer can reach. Another account on a shared computer, such as a Linux login node, can find the port. Every request except a health check needs the service's secret or a short lived key the service handed out itself, such as an app's one time link. The health check answers only that a Biorouter service runs there.

## Related documentation

- [Crew user manual](README.md): all pages.
- [Joining a workspace](joining-a-workspace.md): the fingerprint check.
- [Hosting a workspace](hosting-a-workspace.md): setting the institution.
- [Teams, channels and people](teams-channels-and-people.md): creating channels.
- [Messages and files](messages-and-files.md): refused files.
- [Agents and chat access](agents-and-chat-access.md): choosing a model.
- [Connections and troubleshooting](connections-and-troubleshooting.md): server identity screens.
- [Command line](command-line.md): every option.
- [Administration](administration.md): SSH settings and the history file.
- [Data privacy and protected health information](../security/data-privacy-and-phi.md): models for sensitive data.
