# Teams, channels and people

> **What this is.** How to create and manage teams and channels in Crew, how to add and remove people, and how names and avatars work.
> **Status:** Current.
> **Audience:** Lab members who use Crew in the Biorouter desktop app. No terminal experience is needed.

A Crew workspace is organized like Slack. The workspace holds teams. Each team holds channels. You talk and share files in channels. Crew has no direct messages: every conversation happens in a channel. This page shows how to create teams and channels, how to get people into them, and how Crew shows each person's name.

In the labels quoted on this page, text in braces stands for a name Crew fills in. For example, "Members of {team}…" appears as "Members of Imaging Group…" for a team named Imaging Group.

## Words used on this page

| Word | Meaning |
|---|---|
| Workspace | Your lab's shared space. It runs on a lab server. |
| Host | The person who runs the workspace on the server. Shown with a "Host" badge in the workspace's People list. |
| Team | A group of channels and the people in them. |
| Team owner | The person who created the team. This never changes. |
| Channel | A place for messages and files. Every channel belongs to one team. |
| Channel owner | The person in charge of one channel. At first, the person who created it. Shown with a "Channel owner" badge. |
| Username | Your account name on the lab server, shown as `@username`. It identifies you. |
| Display name | The name you choose for others to see. It is optional. |

The host and a channel owner are two different roles. One person often holds both.

## Who can do what

| Action | Who can do it |
|---|---|
| Create a team | Anyone in the workspace |
| Rename a team | The team owner only. The host cannot. |
| Add people to a team | The team owner and the host |
| Create a channel | Anyone in that channel's team |
| Add people to a channel | The channel owner and the host |
| Remove someone from a channel | The channel owner |
| Rename, archive or hand over a channel | The channel owner |
| Change your display name and initials | You |
| Remove someone from the whole workspace | The host only |

Only a person can add people. An agent working in a channel cannot add anyone.

If your workspace runs an older version of Crew on the server, only an owner can add people, and adding someone sends them an invitation instead. See [If adding sends an invitation](#if-adding-sends-an-invitation).

## Find your way around the sidebar

The Crew sidebar runs down the left side of the window. From top to bottom it shows:

```text
lab ▾                         workspace name; click it for the workspace menu
[connection status]
Imaging Group        + ⋯      a team; + and ⋯ appear when you point at it
  # general
  # journal-club        3     3 unread messages
  Add channel
Add team
(A) Alice Chen  on lab-server the You row: your name and your server
```

- Click a team's name to collapse or expand its channels. A collapsed team still shows the channel you have open.
- A channel with unread messages shows its name in bold and a count. The count stops at "99+".
- A pencil in place of the count means the channel holds a message you started and did not send. See [Messages and files](messages-and-files.md).
- **Archived ({n})** appears under a team that has archived channels. Click it to show them.
- The sidebar lists only channels you are in. Crew has no list of other channels to browse. If you are in a team but did not create it, the last line under the team says "Other channels in {team} appear once someone adds you." To join another channel, ask its owner or the host to add you.

Right click a channel for these items. With the keyboard, move to the channel with the arrow keys and press Shift+F10.

- **Mark as read**. It appears only on the channel you have open, when it has unread messages.
- **Copy channel name** copies the name as `#name`.
- **Copy channel ID** copies an ID that support staff may ask for. You do not need it for daily work.

### Use the keyboard in the sidebar

| Key | What it does |
|---|---|
| Up and Down arrows | Move between teams and channels |
| Home and End | Go to the first or last row |
| Left and Right arrows | Collapse and expand a team. Left on a channel moves to its team. |
| Tab on a team | Reaches its Create channel (**+**) and options (**⋯**) buttons |
| Shift+F10 or the Menu key | Opens the selected channel's menu |

## Teams

### Create a team

Anyone in the workspace can create a team.

1. Open the dialog in one of these ways:
   - Click **Add team** at the end of the team list in the sidebar.
   - Click the workspace name at the top of the sidebar and choose **Create team…**.
   - If you are in no team yet, click **Create a team** on the main screen. A host with no team yet clicks **Create team** in the setup checklist, "Get {workspace} ready".
2. In **Name**, type the team's name. The helper under the field reads "Team names are unique in {workspace}."
3. Click **Create team**.
4. If other people are in the workspace, the dialog asks "Add people to {team}". Click **Choose a person**, pick one person, and click **Add**. Or click **Skip for now**. You can add more people later.

Crew opens the new team's `#general` channel. Every team gets a `#general` channel when it is created, and you own it. It is marked "Restricted" when the workspace is Private for everyone, and "Public-safe" when the workspace allows Public.

A workspace can hold at most 100 teams.

### Team name rules

A team name keeps the capitals and spaces you type. It can be up to 64 characters and must contain at least one letter or number. It can use letters, numbers, spaces and these marks: `- _ . ' & ( ) +`.

Two team names count as the same when they differ only in capitals, spaces, dashes, dots, underscores, or letters that look alike. So "Analysis Lab", "analysis-lab" and "ANALYSIS_LAB" are one name. Crew refuses a name that matches a team you cannot see, too.

| Message | What to do |
|---|---|
| "Team name can’t contain @, #, / or :." | Remove those characters. Crew shows this as you type. |
| "A team with this name, or one that looks like it, already exists in this workspace. Choose a different name." | Choose a different name. |
| "Team name can use letters, numbers, spaces and - _ . ' & ( ) + only." | Remove other symbols. |
| "Team name needs at least one letter or number." | Add a letter or number. |
| "Team name is too long. Choose a shorter name." | Use 64 characters or fewer. |
| "Team name can't look like an ID." | Use words instead of a long code. |
| "Team name can't mix writing systems that look alike, such as Latin and Cyrillic letters." | Type the name in one alphabet. |
| "Team name can't contain invisible, control or formatting characters." | Retype the name instead of pasting it. |
| "Too many name attempts. Try again later." | You chose ten names that were already taken within ten minutes. Wait up to ten minutes, then try again. |

### See who is in a team

1. Point at the team in the sidebar and click **⋯**.
2. Choose **Members of {team}…**.

The dialog "Members of {team}" lists the team owner first, then you, then everyone else by name. If you may add people to the team, **Add people to {team}…** appears under the list. Otherwise a line says who can, for example "Only @alice or the host can add people to {team}." Click **Done** to close.

To see everyone in the whole workspace, click the workspace name at the top of the sidebar and choose **People…**. The Members list shows the host first with a "Host" badge.

### Rename a team

Only the team owner can rename a team.

1. Point at the team, click **⋯** and choose **Rename team…**.
2. Change the name in **Name**. The same rules apply as when you create a team.
3. Click **Rename**.

Renaming keeps the team's history and channels. The old name is free for others at once. If you do not see **Rename team…**, either you did not create the team, or the Crew version on your server cannot rename.

### The team menu

Point at a team and click **⋯** to open it. It holds, in this order:

1. **Members of {team}…**: everyone sees this.
2. **Create channel…**: everyone in the team sees this.
3. **Add people to {team}…**: the team owner and the host see this.
4. **Rename team…**: the team owner sees this.
5. **Copy for support**, which holds **Copy team ID**. You need an ID only when support staff ask for one.

Crew has no way to delete a team, leave a team, or remove someone from a team.

## Channels

### Create a channel

Anyone in a team can create a channel in it.

1. Open the dialog in one of these ways:
   - Point at the team and click **+** ("Create channel in {team}").
   - Click **Add channel** under the team's channels.
   - Click **⋯** beside the team and choose **Create channel…**.
2. In **Name**, type the channel's name. The dialog shows the `#` for you, so do not type one. Under the field, "Will be created as #{name}" shows the exact name Crew will save.
3. Under **Content**, choose one:
   - **Restricted** ("For unpublished or sensitive work"). This is selected each time the dialog opens.
   - **Public-safe** ("Public models may read it").
4. Click **Create channel**.

The new channel opens and you are its owner. A workspace can hold at most 1,000 channels.

> **Warning.** The dialog says: "Everyone in this team can see whether a name is taken, so don’t put patient or sample IDs in channel names." Team names work the same way across the whole workspace. Keep sample, patient and study identifiers out of every name.

### Restricted and Public-safe

The **Content** choice decides which AI models may read the channel. It does not decide who can join.

| Choice | Header badge tooltip | Meaning |
|---|---|---|
| **Restricted** | "Only private models can read it. It doesn’t limit who’s in the channel." | Public models cannot read it. Use it for unpublished or sensitive work. |
| **Public-safe** | "Public models may read it when the workspace allows." | Public models can read it, but only if the workspace allows Public. |

You cannot change this choice after the channel exists. To change it, create a new channel. For what Private and Public mean, read [Privacy and security](privacy-and-security.md).

### Channel name rules

Crew turns what you type into a channel name. It makes every letter lowercase and turns each run of spaces, dots and dashes into one dash. So "Data Analysis" becomes `#data-analysis`. Always check the "Will be created as" line before you click **Create channel**.

A channel name can be up to 80 characters. It can use lowercase letters, numbers, dashes and underscores, and must start with a letter or number. Names only need to be unique within one team, so two teams can each have `#methods`.

| Message | What to do |
|---|---|
| "Channel name can’t be empty." | Type a name. |
| "Channel name can’t contain @, #, / or :." | Remove those characters. |
| "Channel name can use lowercase letters, numbers, hyphens and underscores only." | Remove other symbols. |
| "Channel name must start with a letter or number." | Start with a letter or number. |
| "Channel name is too long. Choose a shorter name." | Use 80 characters or fewer. |
| "Channel name can’t look like an ID." | Use words instead of a long code. |
| "A channel with this name, or one that looks like it, already exists in this team. Choose a different name." | Choose a different name. Archived channels keep their names, so an archived channel's name stays taken. |
| "Channel name general is reserved for the team's first channel." | Every team already has `#general`. Choose another name. |
| "Too many name attempts. Try again later." | Wait up to ten minutes, then try again. |

### The channel header

The band across the top of an open channel shows, from left to right:

- The channel name. Click it to open the channel menu.
- A **Restricted** or **Public-safe** badge. Click it to open the channel's About tab.
- An "Archived" badge, if the channel is archived.
- On the right side: the agent access chip, up to three member avatars with a count, and **Channel details**. Click the avatars to see the channel's members. Click **Channel details** to open or close the details pane.

For the agent access chip, read [Agents and chat access](agents-and-chat-access.md).

### The channel menu

Click the channel name in the header to open it. It holds:

- **Channel details**, **Members**, **Files** and **Agent access**. Each opens the details pane on that tab.
- **Add people…**: channel owner only.
- **Mark as read** and **Refresh channel**. After a refresh, the header shows "Up to date" for a moment. See [Messages and files](messages-and-files.md).
- **Copy channel name**: copies the name as `#name`. The item reads "Copied" and the menu closes.
- For the channel owner only: **Rename…**, **Transfer ownership…** and **Archive channel…**.
- **Copy for support**, which holds **Copy channel ID**.

Only the channel owner sees the owner items.

### The details pane

The details pane opens on the right side. It has four tabs: **About**, **Members**, **Files** and **Agent access**. Press Escape while you are in the pane, or click **Close details**, to close it. In a narrow window the pane covers the channel. Click **Back to #{channel}** to return.

The **About** tab lists:

| Row | What it shows |
|---|---|
| **Name** | The channel's name. The owner sees **Rename…** here. |
| **Who can read** | "Private models only" for a Restricted channel, or "Any model the workspace allows" for a Public-safe one. |
| **Owner** | The channel owner. The owner sees **Transfer ownership…** here. While an offer waits, it reads "Offered to {person} · waiting". |
| **Created by** | The person who created the channel. |
| **Team** | The team the channel belongs to. |
| **IDs for support** | A folded section holding **Copy channel ID**. |

The owner also sees a **Danger zone** heading with **Archive channel…**.

The **Members** tab is described in [See who is in a channel](#see-who-is-in-a-channel). The **Files** tab is described in [Messages and files](messages-and-files.md). The **Agent access** tab is described in [Agents and chat access](agents-and-chat-access.md).

### Rename a channel

Only the channel owner can rename it, and only while it is not archived.

1. Open the channel menu and choose **Rename…**, or click **Rename…** on the About tab.
2. Type the new name. Check the "Will be created as" line.
3. Click **Rename**.

The channel keeps its messages, files and members. The same name rules apply as when you create a channel. If you do not see **Rename…**, the Crew version on your server cannot rename.

### Hand a channel to someone else

A channel has one owner. The owner can offer the channel to another member of it.

1. Open the dialog in one of these ways:
   - Open the channel menu and choose **Transfer ownership…**.
   - Click **Transfer ownership…** on the About tab.
   - On the Members tab, point at a person, click **⋯** and choose **Make owner…**. This picks that person for you.
2. In **New owner**, choose the person. The helper reads "They’ll need to accept."
3. Click **Offer ownership**.

A message says "Ownership offered to {person}". The About tab shows "Offered to {person} · waiting" until they answer. If nobody else is in the channel, the dialog says "No one else in this channel can take it over yet." Add someone first.

The person you chose sees a note above their message box in that channel: "{owner} offered you ownership of #{channel}." They click **Accept ownership** to take it.

> **Warning.** When the new owner accepts, the previous owner leaves the channel and loses access to its messages and files. If you want to stay, ask the new owner to add you back with **Add people…**.

Only the current owner can hand over a channel. The host cannot take over or hand over a channel they do not own.

### Archive a channel

Archive a channel when the work in it is finished. Only the channel owner can archive it.

1. Open the channel menu and choose **Archive channel…**, or click **Archive channel…** under **Danger zone** on the About tab.
2. The dialog asks "Archive #{channel} for everyone?" and says "Nobody can post in it after this. Its history stays readable."
3. Click **Archive channel**.

After archiving:

- The header shows an "Archived" badge.
- The message box is replaced by "This channel is archived."
- The channel moves under **Archived ({n})** in the sidebar.
- Nobody can post, rename it, or add people to it.
- Its name stays taken in the team.

> **Warning.** Archiving cannot be undone. Crew has no way to restore an archived channel.

## Members and adding people

### See who is in a channel

Open the **Members** tab: click the member avatars in the channel header, or choose **Members** from the channel menu.

- The count at the top, "{n} members", counts the channel's members, not the team's.
- The list shows the owner first with a "Channel owner" badge, then you (marked "you"), then everyone else by name. Former members come last, marked "former member".
- The owner sees **Add people…** at the top. Everyone else sees "Ask {owner} to add people."

Point at a person and click **⋯** ("More actions for {person}") for these items:

- **Copy username** copies their username without the `@`.
- For the owner, on anyone else: **Make owner…** and **Remove from #{channel}…**.
- **Copy for support**, which holds **Copy person ID**.

### How adding works

- Adding someone to a team also puts them in the team's `#general`.
- A channel can take only people who are already in its team. Add a person to the team first. You can tick the team's other channels in the same step.
- Adding is immediate. The person does not need to accept, because they agreed to take part when they joined the workspace.
- You can add only people who have already joined the workspace. To bring in someone new, the host invites them first. See [Hosting a workspace](hosting-a-workspace.md).
- The host can add people to any team and channel they can see, including ones they do not own. An owner can add people to their own team and their own channels.

### Add people to a team

You need to be the team owner or the host.

1. Point at the team in the sidebar, click **⋯** and choose **Add people to {team}…**.
2. Tick the people to add. Type in "Search by name or @username" to narrow the list. **Select all ({n})** ticks everyone shown.
3. Under **Also add to**, choose channels:
   - `#general` is ticked and cannot be unticked. It is marked "comes with the team".
   - The team's other channels that you own start ticked.
   - If you are the host, other channels you can see are listed unticked.
4. Click **Add**, or **Add {n} people** when you ticked more than one.
5. Read the summary line at the top of the dialog. Then click **Done**.

Crew adds each person in turn. A problem with one person does not stop the others. If one ticked channel is refused for a person, that person is not added to anything, and the summary says why.

### Add people to a channel

You need to be the channel owner. The host can add people to a channel they do not own when adding those people to its team: tick the channel under **Also add to**.

1. Open the dialog in one of these ways:
   - Open the channel menu and choose **Add people…**.
   - On the Members tab, click **Add people…**.
   - In the channel's welcome message at the top of its history, click **Add people**.
2. Tick the people to add. Only members of the channel's team are listed.
3. Click **Add** or **Add {n} people**.
4. Read the summary line, then click **Done**.

### What the summary line means

| Summary | Meaning |
|---|---|
| "Added {people} to #{channel}." | They are in the channel now. |
| "Added {people} to {team}. They can now see #general and …" | They are in the team and the channels named. |
| "{person} is already in {place}." | Nothing changed for that person. |
| "Couldn’t add {people}: {reason}" | Those people were not added. The reason follows the colon. See [Troubleshooting](#troubleshooting). |
| "Invited {people}. They’ll see it in Crew and need to accept." | Your server runs an older Crew. See [If adding sends an invitation](#if-adding-sends-an-invitation). |

People are listed by username, for example "@ana and @raj".

### When there is nobody to add

When nobody can be added, the dialog says why and lists who is already there under "Already in {place}":

| Note | What to do |
|---|---|
| "No one else has joined {workspace} yet." | The host invites people first. The host sees **Invite people to {workspace}…** here. |
| "Everyone in {workspace} is already here." | Everyone is already in this team. |
| "No one else is in {team} yet." | Add people to the team first. The team owner and host see **Add people to {team}…** here. |
| "Everyone in {team} is already here." | Everyone in the team is already in this channel. |
| "Invited to {workspace}, not joined yet: {names}." | These people were invited but have not joined yet. Add them once they join. |

### Add someone who joined recently (host)

When the host lets someone in, the Let in dialog offers to add them to a team at once. See [Hosting a workspace](hosting-a-workspace.md). If you closed that dialog first, the person stays listed in the sidebar under "Joined, not in your teams", as "{person} · joined".

1. Click **Add to a team…** on that row.
2. If you have several teams, choose one from the menu.
3. The Add people dialog opens for that team. Continue from step 2 of [Add people to a team](#add-people-to-a-team).

The row disappears once the person is in one of your teams.

### When someone adds you

- If Crew is open on that workspace, a notice appears: "{person} added you to #{channel}". It names the channel's owner.
- The channel appears in your sidebar under its team.
- At the top of a channel's history, Crew shows "Welcome to #{channel}" and "{person} created this channel." Everyone except the host also sees "Ask {host} to add you to other channels."

If you are in no team yet, the main screen shows "You’re in {workspace}" and "Ask {host} to add you to a team." You can wait for the host, or click **Create a team** to start your own.

### If adding sends an invitation

A workspace that runs an older version of Crew on the server invites people to teams and channels instead of adding them. On such a workspace:

- Only the owner can add people. The host cannot add people to someone else's team or channel.
- The summary reads "Invited {people}. They’ll see it in Crew and need to accept."
- The team owner sees "{n} invited" beside the team name until people accept. Pointing at it shows "Invited, not accepted yet: {names}".
- The invited person sees an "Invitations" section at the top of their sidebar. Each row shows the team or channel, "from {person}", and a **Join** button. If they are in no team yet, the main screen shows "You’re invited to {team}" with **Join {team}**.

### Remove someone from a channel

Only the channel owner can remove someone.

1. Open the **Members** tab.
2. Point at the person, click **⋯** and choose **Remove from #{channel}…**.
3. The dialog asks "Remove {person} from #{channel}?" and says "They’ll lose access to its messages and files. You can invite them again."
4. Click **Remove**.

The removed person's copy of the channel closes with "You no longer have access to #{channel}, so it was closed." If they had an unsent message there, it adds "Your unsent draft for it was cleared." They stay in the team and the workspace. You can add them back later with **Add people…**.

You cannot remove yourself from a channel you own. To leave it, hand it to someone else: you leave the channel when they accept (see [Hand a channel to someone else](#hand-a-channel-to-someone-else)). Crew has no Leave item. To leave a channel you do not own, ask its owner to remove you.

Removing someone from a channel does not remove them from the workspace. Only the host can do that.

### Remove someone from the workspace (host)

Only the host can remove a person from the whole workspace. The host cannot be removed.

1. Click the workspace name at the top of the sidebar and choose **People…**.
2. In the Members list, point at the person, click **⋯** and choose **Remove from {workspace}…**.
3. The dialog asks "Remove {person} from {workspace}?" and says "Removes all of their devices and agent access. Their messages stay in history."
4. In "Type {username} to confirm", type the person's username exactly, with the same capitals. If it does not match, Crew shows "Type the exact username to remove access."
5. Click **Remove from {workspace}**.

On the removed person's computer, Crew shows "You’re no longer in {workspace}". Their messages stay, marked " · former member". Removing a person also ends the access of every chat and agent task in the workspace.

> **Note.** Ask the person to hand over the channels they own before you remove them. A removed person stays the owner of their channels, and only a channel's current owner can rename, archive or hand it over. The host can still add people to those channels.

### Membership changes end agent access

Some changes end the access of every chat and agent task in the workspace, including chats that work in other channels. These changes are:

- adding someone to a team or channel
- removing someone from a channel
- removing someone from the workspace
- archiving a channel
- offering or accepting channel ownership

The next time an affected chat tries to work in Crew, it says that Crew settings changed since it was given access. To continue, choose **Grant access again** in that chat, or start a new chat. Renaming a team or channel does not end access. Read [Agents and chat access](agents-and-chat-access.md).

## Profiles, names and avatars

### Your username

Your username is your account name on the lab server, for example `@crew_alice`. Crew takes it from the server and you cannot change it in Crew. Your username identifies you. The host invites you by it, and the command line finds people by username only.

To copy it, click your name at the bottom of the sidebar and choose **Copy my username**. Crew copies it without the `@`.

### Set your display name

A display name is the name others see beside your messages, for example "Alice Chen". Until you choose one, people see your username.

1. Click your name at the bottom of the sidebar (the You row).
2. Choose **Edit profile…**.
3. In **Display name**, type your name.
   - If your server account has a full name and you have not chosen a name yet, Crew fills it in with the note "Filled in from your account on {server}. Save to use it." Crew does not use it until you click **Save profile**.
   - Otherwise, if your server account's name differs from what is in the field, a link **Use “{name}”** fills it in.
4. Optionally, type up to 12 characters in **Initials (optional)**. See [Avatars](#avatars).
5. Click **Save profile**.

The dialog also shows "Your username: @{username}". You cannot edit it there.

**Edit profile…** is unavailable until you have joined and Crew has verified the connection. The menu shows why, for example "Available once the connection is verified".

When you first join a workspace, Crew may offer your server account's name above the message box: "Use “{name}” as your name in {workspace}?". Click **Use** to save it, **Edit…** to change it first, or **Dismiss** to keep your username. Crew never sets the name without your click. The host sees the same offer in the setup checklist as "Your name: Use “{name}”?".

> **Note for hosts.** Set your display name before you invite people. Each invitation carries your name as it was when you created the invitation.

### Display name rules

- 1 to 64 characters, with at least one letter or number.
- No `@` or `#`. Crew shows "Display name can’t contain @ or #." as you type.
- It cannot be another member's username: "That name is another member's username. Choose a different name."
- Display names do not have to be unique. Two people can both be "Sam Park".

A display name gives no extra rights. Crew and the command line never pick a person by display name.

To show only your username again, type your username in **Display name** and click **Save profile**.

### How Crew shows a person

| Where | How a person appears |
|---|---|
| Beside a message, and at the top of a menu | Display name, then `@username` in lighter text |
| In member lists, the About tab, confirmations and notices | Display name and username, for example "Bob Lee (@bob)" |
| Someone with no display name | `@bob` alone, everywhere |
| Two people with similar display names | Always with their username, for example "Sam Park (@spark)" |
| An agent | "Bob Lee's agent", or "Your agent" for yours |
| Someone who has left the workspace | Their name in lighter text, followed by " · former member" |
| Someone Crew cannot identify | "Unknown member" |

A former member's messages stay in the history. Former members do not appear in the Add people list. Crew refuses to add one with "@{username} isn't a member of this workspace any more. Invite them to the workspace first."

### Avatars

Each person has a round avatar. Agents have a square avatar with a robot.

| Part | What it shows |
|---|---|
| Letter | Without chosen initials, the avatar shows one letter, the same at every size. It is the first letter of your display name ("Alice Chen" shows "A"). Without a display name, it comes from your username. A username made of parts, such as `crew_alice`, uses its last part, so it shows "A". |
| Initials | If you type initials in **Edit profile…**, the avatars beside messages show the first two characters. The smaller avatars in the sidebar and member lists show only the first. Clear the field and save to go back to the letter. |
| Color | Each person's avatar has one of eight colors. Crew picks it from the username, never from the display name, so nobody can take another person's color by choosing a name. Two people can share a color. The name and `@username` beside the avatar tell them apart. |

### The You row

The bottom of the sidebar shows your avatar, your name and "on {server}", where {server} is the lab server's name. Click it for:

- **Edit profile…**
- **Keys and security…**. See [Privacy and security](privacy-and-security.md).
- **Copy my username**

## Do the same from the command line

Every task on this page also works in a terminal. Add `--connection <name>` when you have saved more than one workspace. Put a channel name that starts with `#` in quotes, because an unquoted `#` starts a comment in the shell.

| Task | Command |
|---|---|
| List teams | `biorouter crew teams list` |
| Create a team | `biorouter crew teams create "Imaging Group"` |
| Rename a team you created | `biorouter crew teams rename "Imaging Group" "Imaging Core"` |
| List channels | `biorouter crew channels list --team "Imaging Group"` |
| Create a channel | `biorouter crew channels create --team "Imaging Group" journal-club --classification restricted` |
| Rename a channel you own | `biorouter crew channels rename '#journal-club' reading-group` |
| Archive a channel | `biorouter crew channels archive '#journal-club'` |
| List people in the workspace | `biorouter crew members` |
| Add a person to a team and a channel | `biorouter crew members add @bob --team "Analysis Lab" --channel '#methods'` |
| Remove a person from a channel you own | `biorouter crew remove-member '#methods' @bob` |
| Offer a channel you own | `biorouter crew ownership offer '#methods' @bob` |
| Accept an offer | `biorouter crew ownership accept '#methods'` |
| Show or set your profile | `biorouter crew profile show`, `biorouter crew profile set "Bob Lee" --avatar BL` |

When two teams have a channel with the same name, name the team too, for example `analysis-lab/methods`. For every option, read [Command line](command-line.md).

## Troubleshooting

| What you see | Why | What to do |
|---|---|---|
| A menu item is grey with "Available once the connection is verified" | Crew has not finished checking the connection. | Wait. If it stays, read [Connections and troubleshooting](connections-and-troubleshooting.md). |
| You cannot find a channel someone mentioned | Crew lists only channels you are in. | Ask the channel owner or the host to add you. |
| "@{username} isn't in this channel's team yet. Add them to the team first." | A channel takes only members of its team. | Add the person to the team first. Tick the channel at the same time. |
| "You can only add people to channels you own. Uncheck #{channel} and try again." | You ticked a channel you do not own. | Untick it and click **Add** again. |
| "#{channel} is archived, so no one can be added to it." | The channel is archived. | Add the person to another channel. |
| "Only the team's owner or the workspace host can add people to it." | You are not the owner or the host. | Ask the owner or the host. |
| "@{username} isn't a member of this workspace any more. Invite them to the workspace first." | The person was removed from the workspace. | Ask the host to invite them again. |
| "@{username}'s account on this server changed since they joined, so they can't be added. The host can remove @{username} and invite them again." | The person's server account was renamed or replaced. | Ask the host to remove the person from the workspace and invite them again. |
| "The person you chose no longer has that username. Refresh and choose again." | The list you chose from was out of date. | Close the dialog, open it again, and choose again. |
| "One of the chosen channels isn't in this team. Refresh and choose again." | A ticked channel changed while the dialog was open. | Close the dialog, open it again, and choose again. |
| "You no longer have access to #{channel}, so it was closed." | The owner removed you from the channel. | Ask the owner if this was a mistake. |
| A chat says Crew settings changed since it was given access | Membership or privacy changed in the workspace. | Choose **Grant access again** in that chat. See [Agents and chat access](agents-and-chat-access.md). |
| **Rename team…** or **Rename…** is missing | You are not the owner, or your server runs an older Crew. | Ask the owner, or ask the host about updating Crew. |

## Related documentation

- [Crew user manual](README.md): the index of every page in this manual.
- [Getting started](getting-started.md): what you need, and the parts of the Crew window.
- [Hosting a workspace](hosting-a-workspace.md): inviting people, letting them in, and adding them to a team in the same step.
- [Joining a workspace](joining-a-workspace.md): what a new member sees before and after the host lets them in.
- [Messages and files](messages-and-files.md): writing messages, unread counts, drafts and the Files tab.
- [Agents and chat access](agents-and-chat-access.md): the Agent access tab, and granting access again after a change.
- [Privacy and security](privacy-and-security.md): Private and Public, and what Restricted and Public-safe channels allow.
- [Connections and troubleshooting](connections-and-troubleshooting.md): what the connection status means when menu items are unavailable.
- [Command line](command-line.md): every `biorouter crew` command and option.
- [Administration](administration.md): server requirements, where Crew keeps its data, and workspace limits.
