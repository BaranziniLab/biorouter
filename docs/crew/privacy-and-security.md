# Privacy and security

> **What this is.** The Crew manual page on privacy and security. It explains Private and Public, institutions, "Restricted" and "Public-safe" content, where each rule is enforced, how Crew checks the identity of the server and the workspace, which files Crew refuses to share, and what other people can see.
> **Status:** Current. Checked against the Crew code on 2026-09-25.
> **Audience:** Lab members who use Crew in the Biorouter desktop app, and hosts who run a workspace. The command line section also serves IT staff.

Privacy in Crew decides which AI models may read your lab's messages and files. It never decides which people can see them. People see a channel because they are members of it, and they see a workspace because the host let them in. This page explains the privacy settings, what each one allows, and the security checks Crew makes on your behalf.

In this manual, a word in braces stands for a name that Crew fills in. For example, "Only private models can read {workspace}." appears on your screen as "Only private models can read chen-lab." when your workspace is called `chen-lab`.

## Terms on this page

| Term | Meaning |
|---|---|
| Private model | An AI model that runs on your own computer, or one that your institution runs. In Biorouter's model provider settings, private models are on the **Local** and **Institutional** tabs. The model picker marks one with a padlock and "Private", often followed by its institution, for example "Private · UCSF". |
| Public model | An AI model from a commercial service outside your institution. In Biorouter's model provider settings, these are on the **Public** tab. The model picker marks one "Public". |
| Local model | A private model that runs on your own computer, such as one from Llama Server or Ollama. Crew treats a local model as approved for every institution. |
| Your connection | This computer's saved link to one workspace. It has its own privacy setting, which you choose. |
| Workspace privacy | The setting the host chooses for everyone in the workspace. |
| Institution | A short lowercase ID for your organization, such as `ucsf` or `sdsc`. A Private workspace uses it to decide which private models are approved. |
| "Restricted" | A label on a channel or a message. Only private models may read it. It does not limit which people can read it. |
| "Public-safe" | A label on a channel. Public models may read it, but only when both the workspace and your connection allow Public. |
| Host | The member who runs the workspace on the server under their own server account. |
| SSH | Secure Shell, the encrypted login that lab servers use. Crew connects to the workspace server over SSH. |

## Privacy is about models, not people

In testing, people often read "Private" as a limit on who could join, and "Restricted" as a limit on who could be in a channel. Neither is true. The rules are:

- Who can see a workspace: only people the host lets in.
- Who can see a channel: only the channel's members.
- Which AI models can read a channel: decided by the privacy settings and the channel's label.

A Public connection does not change what you can read. You keep every channel you are in. Only the set of models that may read those channels through your agent changes.

## Two settings decide your privacy

Two settings apply to you in each workspace.

| Setting | Values shown | Who changes it | Where it is kept |
|---|---|---|---|
| Your connection | "Private" or "Public" | You, for your own connection on this computer | On your computer, in your saved connection |
| Workspace | "Private for everyone" or "Allows Public" | Only the host | On the server, in the workspace |

The privacy that applies to you is Private when either setting is Private. It is Public only when your connection is Public and the workspace allows Public. Crew on the server applies the same rule to every agent task that uses a public model.

A new workspace always starts as "Private for everyone", with no institution set.

| Your connection | Workspace | Privacy that applies to you | The reason the privacy panel gives |
|---|---|---|---|
| Private | Private for everyone | Private | "Private because both your connection and {workspace} are Private." |
| Public | Private for everyone | Private | "Private because {workspace} is Private for everyone." |
| Private | Allows Public | Private | "Private because your connection is Private." |
| Public | Allows Public | Public | "Public because your connection is Public and {workspace} allows it." |

### What each privacy allows

When the privacy that applies to you is Private:

- No public model may read the workspace through your connection.
- A private model may read it only if it is approved for the workspace's institution. A local model is always approved.
- If the workspace has no institution set, no agent may work in a Private workspace at all. People can still chat and share files.

When the privacy that applies to you is Public:

- Public models may read "Public-safe" channels that you are a member of.
- Public models may not read a "Restricted" channel, or a "Public-safe" channel that holds any Restricted message, file or shared server path.
- Public models never get your remote work folder on the server.
- A private model that reads Restricted content, or uses your remote work folder, is still held to the institution rules.

## Check a workspace's privacy

The privacy chip is in the status row, the line under the workspace name at the top of the Crew sidebar. It sits at the right end of that row, beside the connection status word. The channel header does not show privacy.

| The chip shows | What it means |
|---|---|
| A padlock and "Private · {institution}", for example "Private · UCSF" | The privacy that applies to you is Private, and an institution is set. |
| A padlock and "Private" | The privacy that applies to you is Private, and no institution is set on your connection or the workspace. |
| "Public", with no padlock | The privacy that applies to you is Public. |
| "Checking privacy…" | Crew is confirming your privacy with the workspace. Its tooltip reads "Checking privacy: Crew is confirming this connection’s privacy with the workspace. It shows here once confirmed." |
| Nothing | Crew is still connecting, or you are offline, reconnecting, not let in yet, or unable to connect. The chip appears once your connection is verified. |

The chip shows only a privacy that the workspace has confirmed. After you change a setting, the chip changes only when the workspace confirms the new setting.

The institution in the chip is the workspace's institution when the workspace has one. Otherwise it is your connection's institution. The chip shows the institution only when the privacy is Private.

Screen readers announce the chip as "Privacy: Private · {institution}", "Privacy: Private" or "Privacy: Public".

### Open the privacy panel

1. Choose the privacy chip. You can also press Enter or Space when the chip has focus.
2. The privacy panel opens. Focus lands on the panel itself, not on a button, so a stray Enter does not change anything.
3. Press Escape, or click outside the panel, to close it.

The panel shows, from top to bottom:

1. The chip's text as its title.
2. One summary sentence:
   - "Only private and {institution}-approved models can read {workspace}."
   - "Only private models can read {workspace}."
   - "Public models can read public-safe channels in {workspace}. Restricted channels stay private."
3. Three facts: "Your connection" ("Private" or "Public"), "Workspace" ("Private for everyone" or "Allows Public") and "Institution" (the institution, or "Not set").
4. A line that gives the reason from the table above. For a member who is not the host, it adds "Only the host can change {workspace}." and "Only people {host} lets in can see it." For the host, it adds "Only people you let in can see {workspace}."
5. The actions that apply to you.

This sketch shows the panel for a member of a Private workspace:

```text
 Private · UCSF
 Only private and UCSF-approved models can read chen-lab.
 ----------------------------------------------------------
 Your connection   Private
 Workspace         Private for everyone
 Institution       UCSF
 ----------------------------------------------------------
 Private because both your connection and chen-lab are Private.
 Only the host can change chen-lab. Only people @alice lets in
 can see it.
                                                    Privacy…
```

The actions at the bottom of the panel depend on the two settings:

| Workspace | Your connection | Actions shown |
|---|---|---|
| Allows Public | Private | **Make my connection public…** and **Privacy…**. Under them: "Makes only your connection Public: public models could then read the public-safe channels you can see in {workspace}. Restricted channels stay private." |
| Allows Public | Public | **Make private** and **Privacy…** |
| Private for everyone | Private or Public | **Privacy…** only. Making your connection Public there would change nothing. |

**Privacy…** closes the panel and opens "{workspace} settings" on the **Privacy** tab. You can also open that tab from the workspace menu: choose the workspace name at the top of the Crew sidebar, then choose **Privacy…**.

## Channel and message labels

### Channel labels

Every channel is "Restricted" or "Public-safe". The label appears as a badge beside the channel name.

| Label | Tooltip or hint | Who can read, in the About tab |
|---|---|---|
| "Restricted" | "Only private models can read it. It doesn’t limit who’s in the channel." | "Private models only". Hint: "Marked Restricted, so public models can’t read it." |
| "Public-safe" | "Public models may read it when the workspace allows." | "Any model the workspace allows". Hint: "Marked Public-safe, so public models may read it when the workspace allows." |

When you create a channel, the **Create channel** dialog has a "Content" choice:

- **Restricted**: "For unpublished or sensitive work". This is selected each time the dialog opens.
- **Public-safe**: "Public models may read it".

A new team's `#general` channel is "Restricted" in a workspace that is Private for everyone, and "Public-safe" in a workspace that allows Public.

### Messages marked Restricted

Crew on the server marks a message or file "Restricted" when any of these is true:

- It was posted from a Private connection.
- The workspace was Private for everyone when it was posted.
- The channel is "Restricted".
- It was made from Restricted material, for example an agent result that read a Restricted message.
- An agent posted it from a Private task, or from a task that used a remote work folder.

The Biorouter background service on your computer adds your connection's privacy to every message and upload you send. So a message you post from a Private connection is Restricted, even in a "Public-safe" channel of a workspace that allows Public.

In a "Public-safe" channel, a Restricted message shows a muted "Restricted" label. Its tooltip reads "Only private models can read this message." In a "Restricted" channel the label is not repeated on each message.

> **Note.** A "Public-safe" channel that holds even one Restricted message, file or shared server path is closed to public models. The channel keeps its "Public-safe" badge. The refusal comes when a public model tries to read it. Messages posted while the workspace was Private for everyone stay Restricted after the host allows Public.

### Names are not private

Team names are unique in a workspace, and channel names are unique in a team. Anyone who tries a name that is taken learns that it exists, even for a team or channel they are not in. The **Create channel** dialog says: "Everyone in this team can see whether a name is taken, so don’t put patient or sample IDs in channel names."

The workspace name is also visible outside the workspace. The **Host a new workspace** dialog says: "Anyone who can sign in to this server can see the workspace name."

Keep patient IDs, sample IDs and other identifiers out of workspace, team and channel names.

## Change your connection's privacy

Your connection's privacy is saved on this computer. Crew does not show it to the host or to other members. Your messages and agent tasks do carry it to the server, which is how Crew marks them Restricted, and the host's server account can read that record.

### Make your connection Public

The link to do this appears only when the workspace allows Public and your connection is Private.

1. Choose the privacy chip.
2. Choose **Make my connection public…**.
3. A dialog opens with the title "Make your {workspace} connection public?" and the text "Public models will be able to read public-safe work you can see here. Restricted content stays private. Your unsent draft will be cleared."
4. Type the workspace's name in the box labeled "Type {workspace} to confirm". The name to type is shown in a grey box beside the field. Letter case and spaces at the start or end do not matter.
5. Choose **Make public**. The button stays unavailable until the name matches. Pressing Enter in the box does nothing, so you must choose the button.

To stop, choose **Cancel** or press Escape. Focus starts on **Cancel** when the dialog opens. If Crew refuses the change, the reason appears inside the dialog.

You can reach the same dialog two other ways:

- In "{workspace} settings", on the **Privacy** tab, choose **Make my connection public…**. It appears only when the workspace allows Public.
- In **Connection settings…**, set **Privacy** to **Public** and choose **Save connection**. Saving a Private connection as Public always opens the dialog first.

What changes when you make your connection Public:

- Public models may read the "Public-safe" channels you are in, through your agent. Restricted channels and Restricted messages stay closed to them.
- Your unsent draft is cleared. Crew says "Access or privacy changed, so your unsent draft was cleared."
- Access you gave your chats and tasks through this connection ends. A connected chat then says "Crew settings changed since access was granted. Grant access again from Crew."
- What you can read does not change.

### Make your connection Private

No confirmation is needed to make your connection Private.

1. Choose the privacy chip, then choose **Make private**. This button appears in the panel only when the workspace allows Public and your connection is Public.
2. If your connection already has an institution, the change is sent at once.
3. If it has no institution, an "Institution" field appears in the panel. Type your institution's ID, for example `ucsf`, and choose **Make private**. Pressing Enter also works.

Other ways:

- In "{workspace} settings", on the **Privacy** tab, choose **Make private**. This button appears whenever your connection is Public. If your connection has no institution, a dialog titled "Make your {workspace} connection private" asks for it first.
- In **Connection settings…**, set **Privacy** to **Private**, fill in **Institution**, and choose **Save connection**.

A Private connection must have an institution. If you save one without it, the Biorouter background service refuses with "Choose this private SSH connection's institution before saving".

### Several workspaces on one server

If you have more than one workspace on the same server saved on this computer, they share one privacy setting and one institution:

- If any of them is Private, Crew treats all of them as Private.
- They cannot use different institutions. If they would, Crew refuses to connect with a message such as "You already use this server for {other workspace} ({its institution}). {this workspace} uses {institution}; one computer can't mix institutions on the same server."

The command line's join preview warns about a mixed institution before it saves. The desktop **Join a workspace** dialog does not warn early. You meet the refusal when Crew connects.

## Change the workspace's privacy (host only)

Only the host can change the workspace's privacy. An agent can never change it, not even the host's own agent. Members see "Only the host can change {workspace}’s privacy." on the **Privacy** tab.

To open the settings, choose the privacy chip and then **Privacy…**, or choose the workspace name and then **Privacy…**. The **Privacy** tab of "{workspace} settings" has three rows:

| Row | What it shows | Buttons |
|---|---|---|
| "Your connection" | Your badge, for example "Private · UCSF". This row reads your saved connection. | **Make my connection public…** or **Make private**, for every member, as described above |
| "Workspace" | "Private for everyone" or "Allows Public" | Host only: **Allow Public…** or **Make Private for everyone…** |
| "Institution" | The workspace's institution, or "Not set" | Host only: **Set institution to {id}…**, when no institution is set and your connection has one |

If the host has no institution on their own connection and the workspace has none, the tab says "Add your institution in Connection settings first."

The host buttons stay unavailable until Crew has confirmed the workspace's current settings. If you act too early, Crew says "Crew is still checking this workspace’s privacy. Try again in a moment."

### Allow Public

1. On the **Privacy** tab, choose **Allow Public…**.
2. A dialog opens with the title "Allow Public in {workspace}?" and the text "Members will be able to choose Public. Agents with access will need permission again."
3. Type the workspace's name in the box labeled "Type {workspace} to confirm".
4. Choose **Allow Public**. As with the other typed confirmations, Enter in the box does nothing and focus starts on **Cancel**.

Allowing Public does not make anyone's connection Public. Each member still chooses for their own connection.

### Make the workspace Private for everyone

1. On the **Privacy** tab, choose **Make Private for everyone…**.
2. A dialog opens with the title "Make {workspace} Private for everyone?" and the text "Agents with access will need permission again."
3. Choose **Make Private for everyone**. No typing is needed. The button reads "Processing..." while the change is sent.
4. A message confirms: "{workspace} is now Private for everyone. Agents with access need permission again."

### What every workspace change does

Every change to the workspace's privacy or institution ends all agent access in the workspace, for every member. Each member must grant access again from Crew. Channel labels do not change, and messages that are already Restricted stay Restricted.

The privacy choice in the **Host a new workspace** dialog sets only the host's own connection. The workspace still starts as "Private for everyone". To let members choose Public, the host uses **Allow Public…** afterwards.

## Institutions

### The institution ID

An institution ID is 1 to 64 characters: lowercase letters, numbers, `-` and `_`, starting with a letter or number. Examples are `ucsf` and `sdsc`. Every institution field shows the placeholder "For example, ucsf or sdsc".

If the ID is not valid, the field says one of these:

- In the **Join a workspace** and **Host a new workspace** dialogs: "Use the short ID: lowercase letters, numbers, - or _, like ucsf."
- In **Connection settings…** and the **Make private** dialog: "Use lowercase letters, numbers, hyphens and underscores, starting with a letter or number."

Crew shows an institution by the name that a configured model provider publishes for it, for example "UCSF" for `ucsf`. When no provider publishes a name, Crew shows the ID. Fields you type into, and the confirmation that sets the workspace's institution, always show the ID. So one screen may say "UCSF" and the next `ucsf`. Both mean the same institution.

### Two institution values

There are two institution values, and they must match:

| Value | Who sets it | Where it is kept | Can it change |
|---|---|---|---|
| Your connection's institution | You, in **Join a workspace**, **Host a new workspace**, **Connection settings…**, or the **Make private** steps | On your computer, in your saved connection | Yes |
| The workspace's institution | The host, once | On the server, shared by everyone | No. It is permanent. |

A Private connection must have an institution. When you join, the invitation usually fills it in. If the invitation has none, the **Join a workspace** dialog says "Your invitation didn’t include the lab’s institution. Ask {host} which institution {workspace} uses."

The host's **Host a new workspace** dialog explains the difference after the workspace is created: "Step 1 set {institution} for your connection on this computer. This sets it for {workspace} itself, for everyone who works there."

If the two values differ, Crew refuses agent work. The message reads "Crew aliases have different institutions; use a separately verified cluster connection" or "privacy_denied: connection and workspace institutions differ". To fix it, set your connection's institution to the workspace's in **Connection settings…**.

### Set the workspace's institution (host)

Until the host sets it, people can chat and share files in a Private workspace, but agents cannot work there. The workspace's institution can never be changed or cleared afterwards. To use a different institution, create a new workspace.

Crew offers to set it in four places:

1. **Host a new workspace**, right after **Create workspace**. The dialog asks "Mark {workspace} as a {institution} workspace?" and says "Agents working in {workspace} can then use only models approved for {institution}. This can’t be undone." It also says "Until then, people can chat and share files in {workspace}, but agents can’t work there. You can do this later from Get {workspace} ready, or from Privacy… in the workspace menu." Choose **Mark as {institution}** to set it at once, or **Not now** to do it later.
2. The setup checklist "Get {workspace} ready", row "Confirm the institution". Choose **Set institution to {institution}…**. If your connection has no institution, the row says "Add your institution in Connection settings first." and offers **Connection settings…**. If the workspace allows Public, the row is ticked with "Not needed while the workspace allows Public."
3. A note above the message box. The host sees it while the workspace is Private and has no institution, and the host's connection has one. It says "Mark {workspace} as a {id} workspace?" and "Agents working here can then use only models approved for that institution. This can’t be undone." Choose **Set institution to {id}…**, or **Not now**. **Not now** hides the note for this workspace on this computer only.
4. The **Privacy** tab of "{workspace} settings", row "Institution": **Set institution to {id}…**.

Places 2, 3 and 4 open the same confirmation:

1. The dialog title is "Set {workspace}’s institution to {id}?" and the text is "This can’t be changed later. Private data can then be used only with models approved for {id}."
2. Choose **Set {id} permanently**. No key confirms it: focus starts on **Cancel**, and you must choose the button.

Setting the institution counts as a workspace change, so it also ends all agent access in the workspace.

If anyone tries to change or clear the institution later, Crew on the server refuses with "privacy_denied: workspace institution cannot be cleared or changed; use a new workspace".

### Which models an agent may use

Crew treats the content an agent would read as protected when any of these is true:

- Your connection is Private.
- The workspace is Private for everyone.
- A channel the agent reads is "Restricted", or holds a Restricted message, file or shared server path.
- The agent uses your remote work folder on the server.
- The chat already holds Restricted material from earlier.

Content that meets none of these conditions is not protected. In practice that means the workspace allows Public, your connection is Public, and the agent reads only "Public-safe" channels that hold no Restricted material.

For protected content:

- Public models are refused.
- The workspace must have an institution.
- A Private connection must have an institution, and it must match the workspace's.
- A private model must be a local model, or be approved for the workspace's institution. A private model that does not say which institution approved it is refused.

For content that is not protected, any model may read it. A public model still never gets your remote work folder.

| Where the agent works | Local model | Private model approved for the workspace's institution | Private model approved elsewhere, or stating no institution | Public model |
|---|---|---|---|---|
| Protected content, and the workspace has an institution | Allowed | Allowed | Refused | Refused |
| Protected content, and the workspace has no institution | Refused | Refused | Refused | Refused |
| Content that is not protected | Allowed | Allowed | Allowed | Allowed |

In **Ask my agent**, the model picker shows each model's privacy and, when it is not approved, "Not approved for {institution}". Before you choose **Start my agent and allow posting here**, the pane explains a mismatch and keeps that button unavailable:

- "{model} is approved for {other institution}. {workspace} uses {institution}. Choose a model approved for {institution}, or a local model."
- "{model} doesn’t say which institution approved it. {workspace} uses {institution}. Choose a model approved for {institution}, or a local model."

When you pick a public model for a task that reads a Restricted channel, the pane says "This model is Public, so it can’t read Restricted channels." In that case the start button stays available, and the Biorouter background service refuses the task when you click it.

A chat you connect with `/crew` is judged by the chat's current model. Once connected, the chat stays with that model. To use a different model, start a new chat. See [Agents and chat access](agents-and-chat-access.md).

## Where each rule is enforced

The Crew window shows settings and asks you to confirm changes. It does not decide anything. The Biorouter background service on your computer and Crew on the server make every decision.

| Part | Where it runs | What it decides |
|---|---|---|
| The Crew window | Your computer | Nothing. It shows the privacy the workspace confirmed, asks you to confirm changes, and makes the start button unavailable early for a model that would be refused. |
| The Biorouter background service | Your computer | It checks that a Private connection has an institution, that the saved workspaces on one server share one institution, and that a model may receive content before anything is sent to it. It also adds your connection's privacy to every message and upload, refuses credential files, and checks the server's identity on every connection. |
| Crew on the server | The lab server, under the host's account | Who is a member. What only the host may do. The workspace's privacy and permanent institution. Which messages and files are Restricted. Every agent's access, which it ends on each workspace change. The history record. |

In Crew's acceptance testing, a task started by going around the unavailable start button was still refused by the background service, and nothing was sent to the model.

Crew on the server applies the workspace and connection rules on its own, even when **Privacy tiers** is turned off in Biorouter's settings on your computer.

### Messages you may see

| Message | What it means | What to do |
|---|---|---|
| "Refresh the workspace to verify connection privacy before sending." | Your privacy changed, and Crew has not confirmed the new setting yet. The same sentence ends with "before granting agent access." or "before uploading." for those actions. | Open the channel menu (the channel name at the top of the channel) and choose **Refresh channel**. When the header shows "Up to date", try again. |
| "Private workspace blocks public models" | The workspace is Private for everyone. | Choose a private model. |
| "Restricted Crew context cannot be sent to a public model" | The task would read protected content. | Choose a private model, or leave out the Restricted channel. |
| "Private Crew context cannot be sent to a public model" | A chat connected to Crew tried to use a public model while your connection is Private, or after the chat read Private content. | Use a private model in that chat, or start a new chat. |
| "Public models cannot access remote files/jobs" | A public model asked for your remote work folder. | Choose a private model for work in the remote folder. |
| "Confirm this workspace's institution before granting an agent; unlabelled private workspaces allow human collaboration only" | The workspace has no institution. | Ask the host to set the workspace's institution. |
| "Set this private SSH connection's institution before granting an agent" | Your Private connection has no institution. | Add it in **Connection settings…**. |
| "This conversation contains another institution's context; start a fresh conversation for this workspace" | The chat already read another institution's material. | Start a new chat. |
| "Crew settings changed since access was granted. Grant access again from Crew." | A privacy or institution change ended the access. | Grant access again from Crew. |
| "Your access to {workspace} changed." | The workspace ended your live view after a privacy or membership change. | Choose **Retry** in the connection bar. If the message comes back, ask the host. |

Some refusals from Crew on the server appear in the desktop app with a code at the start, for example "privacy_denied: connection and workspace institutions differ". The command line prints them without the code.

## Keys, fingerprints and codes

Crew uses three values that look alike. They do different jobs.

| Value | What it looks like | What it is for | Where you see it |
|---|---|---|---|
| Your code | 16 letters and numbers, such as `7QK2-M9XA-3JTP-WZ4D` | You send it to the host so the host can let this computer in. It never contains the letter U. | The join screen, after you choose **Join {workspace}** |
| Workspace fingerprint | 16 characters in four groups, such as `3F2A 9C1E 77B0 D4E1` | A short summary of the workspace's key. You compare it with the host's copy to check an invitation. | The **Join a workspace** dialog, and the host's screens listed below |
| Device fingerprint | 16 characters in four groups | A short summary of the key this computer uses to sign every request. | **Keys and security…** |

Your code is made on your computer from your device key and the workspace key. The server cannot change it.

### Check an invitation (optional)

A workspace fingerprint lets you confirm that the invitation came from your host's real workspace.

1. In the **Join a workspace** dialog, after Crew reads the invitation, open **Check this invitation (optional)**.
2. It reads "Fingerprint {fingerprint}. This isn’t the code you send; your code appears after you choose Join. To double-check the invitation, ask {host} to read the fingerprint from Crew ({host}’s workspace menu shows it)."
3. Ask your host to read the fingerprint from Crew, for example on a call or in person.
4. If the two do not match, do not join. Ask your host to send the invitation again.

The fingerprint in the **Join a workspace** dialog has no Copy button, because it is not something you send.

The host can find the workspace fingerprint in these places:

- The last step of **Host a new workspace**, with a Copy button and the text "People you invite may ask you to read this to check their invitation. Your workspace menu shows it too."
- The workspace menu header, as "Fingerprint {fingerprint}" with a **Copy** button. It appears only while the status is "Connected".
- The dialog that **Let in…** opens: "Fingerprint {person} should see:" and "If {person} asks, read this out. It should match what {person}’s Crew shows."
- **Connection settings…**, under **Workspace details** ("IDs for support").

When the host lets someone in, the dialog says "Paste the code {person} sent you. Only use a code that came from {person}." If a computer shows a code that does not match, the host sees "A computer trying to join as @{username} showed a different code."

### Keys and security

Crew makes a device key on your computer. The private half never leaves your computer. It signs every request you send to the workspace.

To see your keys, open the You menu at the bottom of the Crew sidebar and choose **Keys and security…**. The dialog shows:

- Where your keys are stored: "Stored in your system keychain.", "Stored in an encrypted vault" with "Locked" or "Unlocked", or "Stored in a file on this computer (development profile)." If Crew cannot tell within a few seconds, it says "Couldn’t check where your keys are stored." with a **Retry** button.
- "This device": this computer's device fingerprint, with a Copy button.
- "Devices on your account": each device fingerprint, with "This device" on this computer and a line such as "Added {date} · with an invitation". This list appears when you have more than one device.

Choose **Done** to close the dialog.

### New device notice

When a device you have not reviewed on this computer is added to your account, the connection bar says "A new device was added to your account on {date}." Choose **Review** to open **Keys and security…** and check the list. If you do not recognize a device, tell your host.

This notice is a convenience. Crew remembers the devices you have reviewed only in this computer's app storage, so it is not a security control.

### Use an encrypted vault

By default Crew keeps your keys in your system keychain. You can use an encrypted vault instead, but only for a new Crew profile.

1. Open **Keys and security…**.
2. Open **Use an encrypted vault instead**. It says "Only for a new Crew profile. Existing identities aren’t moved."
3. Choose **Set up vault…**.
4. A system window titled "Initialize Crew encrypted vault" asks for a new passphrase. A second window, "Confirm Crew vault passphrase", asks you to type it again.

If this computer already has Crew identities, the setup is refused with "Vault initialization was refused. Use a fresh Crew profile with no existing identities or credential backend."

Your passphrase is typed into system windows, not into the Crew window. When the vault is locked, the connection bar says "Your Crew vault is locked." Choose **Unlock** and type your passphrase. If that fails, the bar says "Crew couldn’t unlock the vault."

### Secrets Crew keeps apart

| Secret | When you type it | What Crew does with it |
|---|---|---|
| Your server password or verification code | In the "Sign in to {server}" window, when the server asks | Passes it to the server. "Nothing you type is saved." |
| Your vault passphrase | Only if you use an encrypted vault | Uses it to open the vault. It is typed in a system window. |
| An approval secret | Only if Biorouter shows a window titled "Set approval secret for shared BioRouter daemon" | You choose it and keep your own copy, for example in a password manager. Biorouter does not save it. You need it again to reconnect from the desktop app or the command line. |

Each of these is separate. None of them is your computer login password.

## Checking the server's identity

Every server has a key that proves its identity, called its host key. Your computer keeps the host keys of servers you have verified in a known hosts file, usually `~/.ssh/known_hosts`. Crew connects only to servers whose host key is already in that file. Crew never accepts a new or changed host key by itself, and it offers no button to accept one.

Crew also forces strict checking on every connection it makes over SSH. It turns off forwarding of your SSH agent and other forwarding. A jump host (a gateway server your IT team may ask you to go through) must also use strict host key checking. See [Administration](administration.md) for the settings IT staff need.

When the server cannot be verified, the status word reads "Can’t verify server" and the connection bar says "Crew couldn’t verify {server}." The main area then shows one of three screens.

### A server Crew has not seen

Title: "Can’t verify {server} yet". Text: "Crew only connects to servers you’ve already verified."

1. Note the value under "Fingerprint the server offered", when it is shown.
2. Open **How do I verify it?**. It lists three steps:
   1. "Get {server}’s fingerprint from your IT team or your institution’s directory. Check jump hosts too."
   2. "Compare it using your usual SSH setup, then add the full key to your known-hosts file. A fingerprint alone isn’t enough."
   3. "Come back and choose Try again."
3. If you do not use a terminal, send the fingerprint to your IT team and ask them to add the server for you.
4. When the key is in your known hosts file, choose **Try again**.

**Open a terminal here** opens a terminal in your own shell for the comparison. It does not accept anything for you.

### A server whose identity changed

Title: "{server}’s identity changed". Text: "Don’t connect until your IT team confirms this change. Crew won’t connect while the old key is in your known-hosts file."

The screen shows "Fingerprint Crew knew" and "Fingerprint the server offered now". Its only action is **Copy details for IT**. The copied text starts with "Server: {server}" and "Problem: The server’s host key changed since Crew last connected.", followed by the SSH program's own words.

1. Choose **Copy details for IT**.
2. Send the details to your IT team.
3. Do not connect until they confirm the change and update your known hosts file.

### A different workspace

Title: "This isn’t the workspace you joined". Text: "The server answered with a different workspace key. Don’t continue until {host} confirms what changed."

1. Choose **Copy details** and send the details to your host.
2. Do not continue until your host confirms what changed.
3. **Connection settings…** opens your saved connection.

Crew does not reconnect by itself after any of these three screens. It retries by itself only when the network drops.

For signing in and other connection problems, see [Connections and troubleshooting](connections-and-troubleshooting.md).

## Files Crew will not share

### Files from your computer

The Biorouter background service checks every file you share from your computer, however you add it: the Attach menu, drag and drop, paste, or the command line. It refuses credential files, judged after following links and ignoring letter case:

| Kind | File names refused |
|---|---|
| Environment and secrets files | `.env`, `.env.*`, `secrets.*` |
| Private keys and certificates | `*.pem`, `id_rsa`, `id_dsa`, `id_ecdsa`, `id_ed25519`, `*.p12`, `*.pfx` |
| Cloud and tool credentials | `.aws/credentials`, `.codex/auth.json`, `.claude/.credentials.json`, `.config/gh/hosts.yml` |
| Login and password stores | `.netrc`, `_netrc`, `.pgpass`, `.git-credentials`, `.docker/config.json`, `.kube/config` |
| Your own list | Anything named in the machine wide `.biorouterignore` file |

The service also reads the first 64 KiB of each file and refuses it if that part holds a private key, an AWS access key or a model provider key saved in Biorouter. This catches a renamed copy of a credential file.

A public key such as `id_ed25519.pub` is not a credential, and Crew shares it.

When Crew refuses a file, it says "“{name}” looks like a credential file (a password, key or token store), so Crew won't share it."

### Files you save to your computer

When you save a file from Crew, the service refuses to write it into a credential, settings, login or startup location. Examples are folders under your home folder whose names start with a dot, `~/Library` on a Mac (cloud drives inside it are allowed), and `%APPDATA%` on Windows. It never overwrites a program file. The message is "Crew won't save into a credential or settings location. Choose another folder."

### The remote work folder

Your agent can have a remote work folder on the server, set in **Connection settings…** under **Advanced**. Crew on the server refuses a folder that is, contains, or sits inside a protected folder such as `~/.ssh`, `~/.aws`, `~/.config`, `~/.local/bin` or Crew's own folders. It also refuses your home folder itself. The refusal reads "invalid_scope: remote work directory overlaps protected authentication, application state, program or shell startup folders; choose a folder such as ~/crew-work/<workspace>".

Only a private model can use the remote work folder.

## What other people can see

### Other members

Members of a channel see:

- Your display name, your `@username`, and the messages and files you post in that channel.
- The "Restricted" label on your messages in a "Public-safe" channel, when your message is Restricted.
- Posts your agent makes there. They appear as "{your name}'s agent" with an "Agent" badge. Other people do not see which of your chats made the post or its title.
- The Source line at the end of a task result. It names the shared files the task read and who shared them.

Other members do not see:

- Channels they are not in, or the messages and files in them.
- Your connection's privacy setting or institution.
- Your devices, your tasks' settings, or which of your chats are connected. The **Agent access** lists show each person only their own chats and tasks.
- Your computer's files or your other Biorouter chats. Crew reads a file only when you share it, and a chat only when you connect it with `/crew`.

### The host

In the Crew window, the host sees the same channels as any member: only the ones the host is in. The host also:

- Sees the members of the workspace, the people waiting to join, and a warning when a computer shows a different code.
- Can remove a person from the workspace. The confirmation says "Removes all of their devices and agent access. Their messages stay in history."
- Runs Crew on the server under the host's own server account.

The workspace's data is stored on the server in files owned by the host's server account. That account, and any program running under it, can read everything the workspace stores. This includes every channel, including channels the host is not in, every message marked Restricted, every file, the member list and the history record. People with administrator access to the server can read it too. Crew's channel membership protects your work from other members. It does not hide it from the host's account or from server administrators.

The host cannot:

- See your server password or verification codes. You type them on your computer, and Crew does not save them.
- Get your device key or your vault passphrase. They stay on your computer.
- See the privacy you chose when you joined. The **Join a workspace** dialog says "{Host} isn’t told what you chose." After you join, your messages and tasks carry your connection's privacy, as described above.
- Start, stop, instruct or approve your agent, or act as you. The host can end your agent's access only by changing the workspace's privacy or institution, or by removing you.
- Read your computer's files or your other Biorouter chats.

### The history record

Crew on the server keeps the workspace's history in one file under the host's server account. Every change is recorded with who made it, what changed and when. This includes privacy changes, institution changes and membership changes.

- Nothing is removed from the history. The file only grows.
- Removing a person keeps their messages: "Their messages stay in history."
- Archiving a channel keeps its messages: "Its history stays readable."
- Removing a workspace from your computer does not delete your messages on the server: "Your messages stay on the server, and you can add it again."
- The Crew window has no screen for the history record.
- The host's server account can change the file outside Crew. The record is not protected from the host.

When a workspace reaches the size limit, reading still works but changes are refused with "This workspace has grown past the size Crew supports and cannot take more changes. Ask the host about starting a new workspace." See [Administration](administration.md) for the limits.

## Privacy from the command line

The `biorouter crew` commands use the same background service and the same rules. Add `--connection <name>` when you have more than one saved workspace.

| Command | What it does |
|---|---|
| `biorouter crew privacy show` | Shows your connection's privacy, the workspace's privacy, each institution, and each channel's label. |
| `biorouter crew privacy set-personal private --institution ucsf` | Makes your connection Private with institution `ucsf`. |
| `biorouter crew privacy set-personal public` | Makes your connection Public. |
| `biorouter crew privacy set-workspace public` | Allows Public in the workspace. Host only. |
| `biorouter crew privacy set-workspace private --institution ucsf` | Makes the workspace Private for everyone and sets its permanent institution. Host only. |
| `biorouter crew credentials status` | Shows where your keys are stored, and if they are in a vault, whether it is locked. |
| `biorouter crew credentials init`, `unlock`, `lock` | Sets up, opens or closes the encrypted vault. |

Example output of `biorouter crew privacy show`:

```text
Your connection: Private · institution ucsf · policy epoch 2
Workspace: Private for everyone · institution ucsf · policy epoch 4
Channels (1):
  #general · Crew QA Lab · Restricted
Pass the policy epochs to --expected-policy-epoch and --expected-workspace-policy-epoch to refuse a changed policy.
```

> **Warning.** On the command line, `set-personal public` and `set-workspace public` take effect at once. There is no typed confirmation as in the desktop app.

To make a command refuse instead of acting under a privacy you did not expect, add `--expected-mode private` or `--expected-mode public`. It applies to sending, starting tasks, granting access and file transfers. The `--expected-policy-epoch` and `--expected-workspace-policy-epoch` options refuse a task or grant if either setting changed since you last checked. See [Command line](command-line.md) for every option.

## Known limits

- Crew's protections keep your work from other members and from models the rules forbid. They do not hide anything from the host's server account or from the server's administrators.
- Crew has been tested on macOS desktops only. It has not been tested on Windows or Linux desktops.
- A changed host key, a jump host and verification codes were not exercised in live testing. The rules that refuse saving into settings locations, and the longer list of credential files, were checked by automated tests only.

## Related documentation

- [Crew user manual](README.md): the list of every page in this manual.
- [Getting started](getting-started.md): the parts of the Crew window, including the status row and the privacy chip.
- [Joining a workspace](joining-a-workspace.md): the privacy choice and the fingerprint check when you join.
- [Hosting a workspace](hosting-a-workspace.md): creating a workspace, the institution step and letting people in.
- [Teams, channels and people](teams-channels-and-people.md): creating "Restricted" and "Public-safe" channels.
- [Messages and files](messages-and-files.md): sharing files and the credential file check.
- [Agents and chat access](agents-and-chat-access.md): choosing a model for a task and granting or ending chat access.
- [Connections and troubleshooting](connections-and-troubleshooting.md): signing in and connection messages.
- [Command line](command-line.md): every `biorouter crew` command and option.
- [Administration](administration.md): server setup, jump host settings, storage limits and the history file.
- [Data privacy and protected health information](../security/data-privacy-and-phi.md): which model providers suit sensitive research data.
