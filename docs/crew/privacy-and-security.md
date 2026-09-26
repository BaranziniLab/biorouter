# Privacy and security

> **What this is.** The Crew manual page on privacy and security: which AI models may read your work, and what other people can see.
> **Status:** Current. Checked against the Crew code on 2026-09-25.
> **Audience:** Lab members who use Crew in the Biorouter desktop app, and workspace hosts.

Privacy in Crew decides which AI models may read your lab's messages and files, never which people see them. Only people the host lets in see a workspace, and only a channel's members see the channel. The host is the member who runs the workspace on the server. A word in braces, such as {workspace}, stands for a name Crew fills in.

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

Crew does not show your choice to the host or other members. Your messages, uploads and agent tasks carry it to the server, where the host's server account can read it.

### Make your connection Public

1. Choose the privacy chip, then **Make my connection public…**, offered when the workspace allows Public. The **Privacy** tab has the same button, and saving **Public** in **Connection settings…** opens the same dialog.
2. Type the workspace name in "Type {workspace} to confirm", then choose **Make public**.

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

## Messages you may see

| Message | What to do |
|---|---|
| "Refresh the workspace to verify connection privacy before …" | Crew has not confirmed your connection's privacy yet. See [Other messages](connections-and-troubleshooting.md#other-messages). |
| "Private workspace blocks public models", "… cannot be sent to a public model" or "Public models cannot access remote files/jobs" | Choose a private model, or leave out the Restricted channel. In a connected chat, start a new chat. |
| "Confirm this workspace's institution before granting an agent; …" | Ask the host to set the institution. |
| "Set this private SSH connection's institution before granting an agent" | Add it in **Connection settings…**. |
| "privacy_denied: connection and workspace institutions differ", "… one computer can't mix institutions on the same server." or "Crew aliases have different institutions; …" | Use the workspace's institution in **Connection settings…**, for every workspace on that server. |
| "This conversation contains another institution's context; …" | Start a new chat. |

## Keys, fingerprints and codes

You send your code, such as `7QK2-M9XA-3JTP-WZ4D`, to the host to be let in. Your computer makes it, so the server cannot change it. The workspace fingerprint, such as `3F2A 9C1E 77B0 D4E1`, is never sent. To check an invitation, open **Check this invitation (optional)** when you join, and join only if your host reads the same fingerprint from Crew.

Your device key's private half never leaves your computer. **Keys and security…**, in the You menu at the bottom of the Crew sidebar, shows where your keys are stored and each device on your account. If Biorouter asks you to "Set approval secret for shared BioRouter daemon", keep your own copy. You need it to reconnect.

### New device notice

When a device joins your account, the connection bar says "A new device was added to your account on {date}." Choose **Review**, and tell your host about any device you do not recognize.

### Use an encrypted vault

A vault keeps your device keys in a file locked by a passphrase, instead of the system keychain. Set it up before you host or join your first workspace on this computer, because keys already in the keychain do not move. If Crew has already used them, setup fails with "Vault initialization was refused. Use a fresh Crew profile…".

1. Open **Keys and security…** from the You menu.
2. Choose **Use an encrypted vault instead**, then **Set up vault…**.
3. In "Initialize Crew encrypted vault", type a new passphrase. It must differ from the approval secret and be at most 1024 bytes.
4. In "Confirm Crew vault passphrase", type it again.

**Keys and security…** now reads "Stored in an encrypted vault" and offers **Lock**. When the vault is locked, the connection bar says "Your Crew vault is locked." Choose **Unlock** there or in **Keys and security…**, then type the passphrase.

> **Warning.** Nobody can recover a lost passphrase. If the vault files go missing, restore them from a backup. See [Keep device keys in an encrypted vault](command-line.md#keep-device-keys-in-an-encrypted-vault).

## Server identity

Crew reaches the server over SSH and connects only to a server whose key is already in your known hosts file, usually `~/.ssh/known_hosts`. It never accepts a new or changed key. See [Server identity checks](connections-and-troubleshooting.md#server-identity-checks).

## Files Crew will not share

Crew refuses to share credential files such as `.env`, `secrets.*`, private keys, cloud credentials and password stores, even renamed. It shares public keys such as `id_ed25519.pub`. It also refuses to save into a credential or settings folder. See [Messages and files](messages-and-files.md).

Your remote work folder, under **Advanced** in **Connection settings…**, cannot be your home folder or overlap a protected folder such as `~/.ssh` or `~/.aws`. Only a private model can use it.

## What other people can see

Other members see your name, `@username` and posts in channels you share, with their "Restricted" labels. Your agent posts as "{your name}'s agent", and a task's Source line names the shared files it read. Members never see channels they are not in, your privacy, institution, devices or connected chats.

In Crew, the host sees only the channels the host is in, plus the members, people waiting to join, and a warning when a computer shows a different code. The host never sees your password, the verification codes you sign in with, or your keys, and cannot start, stop or approve your agent. Some workspace changes end every agent's access, yours included: adding or removing people, archiving a channel, changing a channel's owner, and changing the workspace's privacy or institution. See [Changes that end agent access](hosting-a-workspace.md#changes-that-end-agent-access).

> **Warning.** The workspace's data sits in files owned by the host's server account. That account, programs running under it and the server's administrators can read everything, including Restricted messages and channels the host is not in.

A history file under the host's account records each change and who made it. Nothing is removed from it, but the host's account can edit it. Crew has no screen for it. See [Administration](administration.md).

## Privacy from the command line

From a terminal, `biorouter crew privacy show` shows these settings, `privacy set-personal` changes your connection and `privacy set-workspace` changes the workspace (host only). Add `--expected-mode private` to make a command refuse rather than act under Public. See [Command line](command-line.md).

> **Warning.** `set-personal public` and `set-workspace public` act at once, with no typed confirmation.

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
