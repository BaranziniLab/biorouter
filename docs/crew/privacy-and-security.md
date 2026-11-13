# Privacy and security

> **What this is.** The Crew manual page on privacy and security: which AI models may read your work, what other people can see, your computers and how to remove one, and what full agent access and open workspaces risk.
> **Status:** Current. Checked against the Crew code on 2026-11-07, after the account ownership change ([design](design/account-ownership.md)) and the second part of the chat-parity build. Updated on 2026-11-12 for coding-agent parity ([design](design/coding-agent-parity.md)).
> **Audience:** Lab members who use Crew in the Biorouter desktop app, and workspace hosts.

Privacy in Crew decides which AI models may read your lab's messages and files, never which people see them. Only people who were let in see a workspace, by the host or in an open workspace by any member, and only a channel's members see the channel. The host is the member who runs the workspace on the server. A word in braces, such as {workspace}, stands for a name Crew fills in.

## What Private and Public allow

A private model runs on your computer or at your institution. Biorouter lists it on the **Local** or **Institutional** tab of its model provider settings and marks it "Private", for example "Private · UCSF". A public model is a commercial service, on the **Public** tab. Local models, from Llama Server or Ollama, are approved for every institution.

Crew decides by a model's privacy, never by its provider. Claude Code and Codex, which run on your own vendor subscription, are public models like any other on the **Public** tab: Crew admits them for a task, a connected chat or a chat that reads Crew as you wherever it admits a public model, and refuses them wherever it refuses one.

A private model, such as Versa, may read a Public workspace too. A chat that is already Private, because it used a private model or read private data, keeps its privacy there: it connects only to a workspace whose host set its institution (unlabelled ones refuse it with "Confirm this workspace's institution before granting an agent"), and everything it posts there is Restricted, so public models never read it. A new chat that has not used a private model yet connects like any other.

In each workspace, your connection is "Private" or "Public". You choose it, and this computer saves it. The workspace is "Private for everyone" or "Allows Public". Only the host changes it, and a new workspace starts "Private for everyone" with no institution. Your privacy is Public only when your connection is Public and the workspace allows it. A Public connection never changes what you can read.

Crew protects what an agent reads when your privacy is Private, when a channel it reads is Restricted or holds anything Restricted, when it uses your remote work folder, or when the chat already holds Restricted material.

| Model | Protected content | Other content |
|---|---|---|
| Public | Refused | Allowed, except the remote work folder |
| Local, or approved for the workspace's institution | Allowed | Allowed |
| Private, approved elsewhere or stating no institution | Refused | Allowed |

Protected content also needs a workspace institution that matches your connection's. In a Private workspace with no institution, people can chat and share files, but no agent can work.

In **Ask my agent**, a model that is not approved shows "Not approved for {institution}", and **Start my agent and allow posting here** stays unavailable. A public model on a Restricted channel is refused when you start. A chat connected with `/crew` keeps its model. A chat that reads Crew as you, with your permission, follows the same rules channel by channel, and with a private model becomes Private once it reads workspace content ([Use Crew from a chat](agents-and-chat-access.md#use-crew-from-a-chat)). See [Agents and chat access](agents-and-chat-access.md).

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

Crew knows you by your account on the lab server ([You are your server account](joining-a-workspace.md#you-are-your-server-account)). Each computer that uses a workspace as you has its own device key for it. The key's private half never leaves that computer, and the key signs every request the computer sends. On a server with a current Crew, a computer that signs in to the server as your account can add its own key to your account by itself, with no code and no other computer. So whoever can sign in to the server as you can use Crew as you: keep your SSH keys and password to yourself.

On a server with an older Crew, you send your code, such as `7QK2-M9XA-3JTP-WZ4D`, to the host to be let in. Your computer makes it, so the server cannot change it. The workspace fingerprint, such as `3F2A 9C1E 77B0 D4E1`, is never sent. To check an invitation, open **Check this invitation (optional)** when you join, and join only if your host reads the same fingerprint from Crew.

### Your computers

**Keys and security…**, in the You menu at the bottom of the Crew sidebar, shows where your keys are stored and, under "Computers enrolled as you", each computer enrolled as you in the workspace, with its fingerprint and when and how it was added, such as "Added November 6" and "by signing in to the server". **This computer** marks the one you are using, and the fingerprint under the heading "This computer" is its key's. A computer the new-computer note below is about is marked **New** until you acknowledge it. From a terminal, run `biorouter crew profile devices` ([Your computers](command-line.md#your-computers)).

When a computer is added to your account, every other computer of yours is told at its next connection, even one that was offline then. A computer that has the workspace open in Crew is told within a few seconds, with no need to leave Crew and come back. The connection bar names it with when and how it was added, in the words **Keys and security…** uses, such as "New computer on your account: {fingerprint}, added {date} by signing in to the server.", or says "{count} new computers on your account." for several. **Review** opens **Keys and security…**, where the new ones are marked **New**. **Acknowledge** marks the computers the note named as seen, so the note goes away. A computer added after the note appeared is not marked seen with them: it stays in the note until you acknowledge it too. `biorouter crew status` names the new computer too, until you mark it seen with `biorouter crew profile devices --acknowledge`.

To remove a computer you do not recognize, or a lost one, on a server with a current Crew:

1. Take its SSH access to the server away first. A computer that can still sign in to the server as you can add itself again. Remove its key from `~/.ssh/authorized_keys` on the server, or change your password there. Ask IT if you are not sure how.
2. In **Keys and security…**, choose **Remove** on the computer's row. The row asks "Remove {fingerprint} from your account?" and repeats the warning: "A lost computer can still sign in as you while it has an SSH key or password for your account. First remove its key from ~/.ssh/authorized_keys on the server, or change your password. Then remove it here."
3. Choose **Remove computer**, or **Keep** to leave it. **Keep** and Escape put the keyboard focus back on the row's **Remove**. Crew says "Removed {fingerprint}."

From a terminal, run `biorouter crew profile revoke-device` with its fingerprint. From a chat, ask it to remove the computer: its card names the fingerprint and gives the same warning, and nothing happens until you choose **Allow once**. A chat can also mark new computers as seen, after its own card lists each one, but reading your computers never marks them seen.

You can remove the computer marked **This computer** too. It is the computer you are using, not a lost one, so its question has no SSH warning: it asks "Remove this computer from your account?" and adds "This computer stops reaching the workspace until you choose Add this computer again." Its button reads **Remove this computer**. Afterwards Crew says "Removed this computer. Choose Add this computer again to use the workspace here.", and the connection bar offers **Add this computer again** ([Add this computer again](joining-a-workspace.md#add-this-computer-again)). Removing a whole person from the workspace stays the host's task.

### Use an encrypted vault

A vault keeps your device keys in a file locked by a passphrase, instead of the system keychain. Set it up before you host or join your first workspace on this computer, because keys already in the keychain do not move. A Linux computer with no keyring service, such as a server you reach only over SSH, needs the vault: Crew never keeps a device key in a plain file, so without one it cannot save a key and says "This computer has no keyring service Biorouter can use…". If Crew has already used them, setup fails with "Vault initialization was refused. Use a fresh Crew profile…".

1. Open **Keys and security…** from the You menu.
2. Choose **Use an encrypted vault instead**, then **Set up vault…**.
3. In "Initialize Crew encrypted vault", type a new passphrase.
4. In "Confirm Crew vault passphrase", type it again.

**Keys and security…** now reads "Stored in an encrypted vault" and offers **Lock**. When the vault is locked, the connection bar says "Your Crew vault is locked." Choose **Unlock** there or in **Keys and security…**, then type the passphrase.

A chat can open the same windows for you. When it needs the vault, it shows an **Unlock** or **Set up** card, and the button opens your computer's own passphrase window. The passphrase goes from that window to Crew on this computer, never to the chat, the model or the chat's history. A chat locks the vault only after you allow its card, because a locked vault stops every chat and Crew task that uses it ([Sign in, unlock and check a server's key](agents-and-chat-access.md#sign-in-unlock-and-check-a-servers-key)).

> **Warning.** Nobody can recover a lost passphrase. If the vault files go missing, restore them from a backup. See [Keep device keys in an encrypted vault](command-line.md#keep-device-keys-in-an-encrypted-vault).

## Server identity

Crew reaches the server over SSH and connects only to a server whose key is already in your known hosts file, usually `~/.ssh/known_hosts`. It adds a new key only when you choose **Trust this key** after you compared its fingerprint with the one IT gave you, and it never accepts a changed key. See [Server identity checks](connections-and-troubleshooting.md#server-identity-checks).

**Trust this key** holds to these rules, in Crew and in a chat alike:

- It is your act: it takes the proof that you are at this computer, and no tool a model calls reaches it.
- It is only for a server this computer knows no key for. When any key is known for it, a changed, revoked or other key, Crew refuses and adds nothing.
- Crew reads the key from the server again and writes only a key whose fingerprint is the one you were shown. The key comes from the server, never from what was typed.
- Crew refuses to read or add a key when your SSH settings reach the server through a jump host or under another name, because a read from this computer could reach another machine. Whatever Crew shows, trust a key only when its fingerprint matches the one IT gave you.

A chat follows the same rules. Its card for an unknown key shows the fingerprint this computer read from the server, and you check it, then choose **Trust this key** or add the key yourself, in a terminal of your own. The chat is told never to add, scan or accept a key, and in **Completely Autonomous** mode a shell command of an agent's that would trust a key nobody checked, such as `ssh -o StrictHostKeyChecking=accept-new`, an `sshfs` mount with that option, or `ssh-keygen -R`, asks you before it runs ([Permission modes](../security/permission-modes.md#what-still-asks-whatever-your-mode)).

### An agent's own shell

A chat with a shell could run `biorouter crew` there and act as you without a card. So Crew refuses every change it makes from an agent's own shell, before anything is sent, and points the agent to `crew__change`, whose card you see ([In an agent's shell](command-line.md#in-an-agents-shell)). Reads still work there. In the same way, `biorouter session attach` run from an agent's shell answers no approval. This is accident prevention, not a boundary: it stops an agent's ordinary command, and a program set on acting as you on your own computer is beyond what Crew guards against. Crew's program on the server does the same for an agent's job there, which cannot add or remove your computers.

### What a chat never sees

When a chat signs you in, unlocks the vault or checks a server's key, your typing never passes through it:

- The **Sign in** card opens the Sign in window. What you type there goes from the window to ssh on the server, with the proof that you are at this computer, and never to the chat, the model, the chat's history or Biorouter's logs.
- The **Unlock** and **Set up** cards open your computer's own passphrase window, which sends the passphrase to Crew on this computer only.
- A step card's words are Crew's own, with the server, login and fingerprint Crew read itself. Nothing the model writes appears on one, so a message in a channel cannot put words on a card.
- A chat with no person to answer, such as one in a `biorouter serve` browser, where Crew does not work, or one through ACP, gets no such card. It names the step for you to take in Crew or a terminal.
- A chat on Claude Code or Codex uses the same cards. Its model is public, so it reads only what a public model may read, and every change it makes asks you first.

## Files Crew will not share

Crew refuses to share credential files such as `.env`, `secrets.*`, private keys, cloud credentials and password stores. It judges each file by its name and by its content, reading the first and last 64 KB of the file, so a renamed key file is refused too. A credential in the middle of a file larger than 128 KB is beyond this check. It shares public keys such as `id_ed25519.pub`. It also refuses to save into a credential or settings folder. See [Messages and files](messages-and-files.md).

With **In a folder** access, your remote work folder cannot be your home folder or overlap a protected folder such as `~/.ssh` or `~/.aws`. Only a private model can use it. With full access the agent can reach those folders, so Crew's secret guard reads each command and file path the way the server would, and refuses one that names a credential file, and it withholds results that carry a key or token ([Full access to the server](#full-access-to-the-server)).

## Full access to the server

**As me on the server**, the full level of [agent access](agents-and-chat-access.md#what-your-agent-may-do-on-the-server), lets your agent run commands as your own server account. It is your choice, for your account only, and it is the default for a connection made by joining or hosting, because agents that compute on the server need it. Choose **In a folder** or **Off** in **Connection settings…** if you do not want it.

What it risks:

- **Your whole account.** The agent can read and change anything your account can, use the network, and start programs that keep running after its access ends. A misled agent could delete files, send data off the server, or replace your own copy of Crew there.
- **Other people's words.** Messages, files and the workspace context written by other members reach your agent. Crew tells it to treat them as information, never as instructions, but a crafted message can still try to steer it. Use a private model you trust, and give tasks that say what to touch.
- **Host accounts.** If you host the workspace from that account, your agent can read the workspace's own files on the server, including channels you are not in.
- **You in Crew.** A program running as your server account is you to Crew. A command of your agent's can add a computer to your account and act as you in the workspace, posting with no "agent" mark, and on the account that hosts the workspace it holds the host's authority. Crew's program on the server refuses to add or remove your computers from inside an agent's job, as a guard against mistakes, but a command that reaches the workspace another way is not stopped. The warning you read before you choose this level says so: "Crew treats anything running as your account as you, so your agent can also add a computer to your account and post in Crew as you, with no agent mark.", and when you host the workspace from that account, "You host {workspace} from this account, so your agent can also read the workspace’s own files on the server, including channels you’re not in. It can also act as the host of {workspace}." The command line and a chat's cards give the same warning wherever they set this level, and so does a chat's card for a task that gets full access ([As me on the server](agents-and-chat-access.md#as-me-on-the-server)). Check **Keys and security…** for computers you did not add ([Your computers](#your-computers)).

What it does not change:

- A public model never gets any server access, whatever the level.
- An agent **In a folder**, or with **Off**, cannot reach the workspace on the server, so it cannot act as you in Crew.
- Each command is announced in the task's steps by its program and length, never its text, and the secret guard still refuses credential files and withholds credentials from results.
- Revoking a chat, or a task's end, stops new commands. A detached job keeps running until it ends or you stop it.

### Open workspaces trust every member

A workspace the host creates is open. In an open workspace every member can rename, archive and delete any team or channel they are in, add and remove people in them, join any of them, invite new people and let them in. Only the host removes someone from the workspace, changes its privacy or governance, renames it or deletes it.

- A new member is added to every team and every channel that is not archived, so they can read everything already posted in all of them, files included.
- Any member can delete a channel or a team for everyone, and nobody can bring it back.
- Adding people in an open workspace does not end agent access, so a task or chat that reads a channel goes on reading it after someone new joins.

The host can switch the workspace to managed at any time ([Choose who manages the workspace](hosting-a-workspace.md#choose-who-manages-the-workspace)), and back. Switching to managed also stops invitations made by members from admitting anyone.

## What other people can see

Other members see your name, `@username` and posts in channels you share, with their "Restricted" labels. Everyone in the workspace sees whether you are online, meaning your computer is connected to the workspace now or made a request in the last three minutes. Your agent posts as "{your name}'s agent", and the Source line Crew adds to each task result and each connected chat's post names the shared files it read. Members never see channels they are not in, your privacy, institution, computers or connected chats.

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
