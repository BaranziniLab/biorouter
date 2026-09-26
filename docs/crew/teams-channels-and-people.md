# Teams, channels and people

> **What this is.** How to use the Crew sidebar, create and manage teams and channels, add and remove people, and set your name.
> **Status:** Current.
> **Audience:** Lab members who use Crew in the Biorouter desktop app. No terminal experience is needed.

A workspace holds teams, and each team holds channels where you post messages and share files. Crew has no direct messages. Text in braces stands for a name Crew fills in, such as {team}.

## Roles and permissions

| Action | Who can do it |
|---|---|
| Create a team, or a channel in your team | Anyone |
| Rename a team | The team owner, who created it |
| Add people to a team | The team owner or the host |
| Add people to a channel | The channel owner, or the host through the team |
| Rename, archive or hand over a channel, or remove people from it | The channel owner only |
| Remove someone from the workspace | The host only |

The host runs the workspace and has a "Host" badge. A channel's owner starts as its creator and has a "Channel owner" badge. Agents cannot add people. Nobody can delete a team, leave one, or remove someone from one.

## Find your way around the sidebar

- Choose a team's name to collapse or expand it. A collapsed team still shows the open channel.
- Point at a team for **+** (new channel) and **⋯** (team menu).
- A bold channel name with a count, up to "99+", has unread messages. A pencil marks an unsent message. See [Unread messages](messages-and-files.md#unread-messages).
- **Archived ({n})** under a team shows its archived channels.
- The sidebar lists only your channels, with no channel browser. Under a team you did not create, it says "Other channels in {team} appear once someone adds you." Ask the channel's owner or the host.
- Right click a channel for **Mark as read** (open channel only), **Copy channel name** and **Copy channel ID**, which support staff may ask for.

### Use the keyboard in the sidebar

Up, Down, Home and End move between teams and channels. Left and Right collapse and expand a team, and Left on a channel moves to its team. On a team, Tab reaches **+** and **⋯**. Shift+F10 or the Menu key opens a channel's menu.

### The You row

The bottom row shows your avatar, name and "on {server}". It opens **Edit profile…**, **Keys and security…** and **Copy my username**, which copies it without the `@`.

## Create and manage teams

### Create a team

1. Choose **Add team** below your teams, or **Create team…** in the workspace menu. With no team yet, the main screen offers **Create a team**.
2. Type a name in **Name** and choose **Create team**.
3. If Crew asks to "Add people to {team}", choose someone and **Add**, or choose **Skip for now**.

The team's `#general` channel opens, and you own it.

### Team name rules

A team name has up to 64 letters, numbers, spaces and `- _ . ' & ( ) +`, including a letter or number. It must be unique in the workspace, including teams you cannot see. Differences in capitals, spaces, dashes, dots, underscores or look alike letters do not count, so "Analysis Lab" and "analysis-lab" are one name.

A refusal says what to change. After ten taken names in ten minutes, Crew says "Too many name attempts. Try again later." Wait up to ten minutes.

> **Warning.** Anyone who tries a taken name learns that it exists. Keep patient, sample and study IDs out of team and channel names.

### See who is in a team

Point at the team, choose **⋯**, then **Members of {team}…**. Under the list you see **Add people to {team}…**, or a line naming who can add people. For the whole workspace, choose the workspace name, then **People…**.

## Create and manage channels

### Create a channel

1. Point at the team and choose **+**, or choose **Add channel** under its channels.
2. Type a name in **Name**, without a `#`. "Will be created as #{name}" shows the name Crew saves.
3. Under **Content**, keep **Restricted** or choose **Public-safe**.
4. Choose **Create channel**. The channel opens, and you own it.

The **Content** choice decides which AI models may read the channel, not who can join. You cannot change it later. See [Privacy and security](privacy-and-security.md).

### Channel name rules

A channel name has up to 80 lowercase letters, numbers, dashes and underscores, and starts with a letter or number. Crew converts what you type, so "Data Analysis" becomes `#data-analysis`. It must be unique in its team, archived channels included, and `general` is reserved. Refusals work as for [team names](#team-name-rules).

### The channel header

The header shows the channel name, a "Restricted" or "Public-safe" badge, an "Archived" badge when archived, an agent access count, member pictures, and **Channel details**. The badge opens the **About** tab, and the pictures open **Members**.

The channel name opens the channel menu. It has **Mark as read**, **Refresh channel**, **Copy channel name**, and the owner's actions described below.

### The details pane

**Channel details** opens the pane, and Escape closes it.

| Tab | What it holds |
|---|---|
| **About** | Name, who can read it, owner, creator and team. The owner also sees **Rename…**, **Transfer ownership…** and **Archive channel…**. |
| **Members** | The channel's members, not the team's. A person's **⋯** menu has **Copy username**, and for the owner **Make owner…** and **Remove from #{channel}…**. |
| **Files**, **Agent access** | Shared files, and your own chats and tasks that can post here. |

### Rename a team or channel

Only the owner can rename, and not an archived channel.

1. Choose **Rename team…** in the team's **⋯** menu, or **Rename…** in the channel menu.
2. Type the new name and choose **Rename**.

History, files and members stay, and the old name is free at once. If the item is missing, you are not the owner, or the server runs an older Crew.

### Hand a channel to someone else

1. As the owner, choose **Transfer ownership…** in the channel menu, or **Make owner…** in a person's **⋯** menu on the **Members** tab.
2. Choose the person in **New owner**, then **Offer ownership**.

You can offer the channel only to another member of it, so add the person to the channel first. With nobody to offer it to, the dialog says "No one else in this channel can take it over yet." Removing the person from the channel cancels the offer.

The **About** tab reads "Offered to {person} · waiting" until the person chooses **Accept ownership** above their message box.

> **Warning.** When the new owner accepts, you leave the channel. To stay, ask them to add you back.

### Archive a channel

1. As the owner, choose **Archive channel…** in the channel menu.
2. At "Archive #{channel} for everyone?", choose **Archive channel**.

The channel moves under **Archived ({n})** with an "Archived" badge. Its members can still read it. Nobody can post in it, rename it or add people, and its name stays taken. Archiving cannot be undone.

## Add and remove people

You can add only people who have joined the workspace. The host invites new people: see [Invite people](hosting-a-workspace.md#invite-people). Adding is immediate. A person added to a team also joins its `#general`. A channel takes only members of its team.

### Add people to a team

1. As the team owner or the host, point at the team, choose **⋯**, then **Add people to {team}…**.
2. Tick people. "Search by name or @username" narrows the list.
3. Under **Also add to**, tick channels. `#general` "comes with the team", and channels you own start ticked.
4. Choose **Add**, or **Add {n} people**.

A summary appears, such as "Added @ana and @raj to {team}. They can now see #general and #methods." Choose **Done**. If a ticked channel is refused for a person, that person is added to nothing, and the summary says why. Others are still added.

### Add people to a channel

1. As the channel owner, choose **Add people…** in the channel menu, on the **Members** tab, or in the channel's welcome message.
2. Tick people. Only members of the team are listed.
3. Choose **Add**. The summary reads "Added {people} to #{channel}."

A host who does not own the channel ticks it in the team's dialog. See [Add people to teams and channels](hosting-a-workspace.md#add-people-to-teams-and-channels).

If the dialog says "No one else is in {team} yet.", add people to the team first. If adding fails, "Couldn’t add {people}: {reason}" says why.

### When someone adds you

The channel appears in your sidebar, and a notice such as "@alice added you to #methods" names its owner. The channel opens with "Welcome to #{channel}" and "Ask {host} to add you to other channels."

### Remove someone from a channel

1. As the owner, open the **Members** tab, choose **⋯** on the person's row, then **Remove from #{channel}…**.
2. Choose **Remove**.

The channel closes on their screen with "You no longer have access to #{channel}, so it was closed." They stay in the team.

Crew has no Leave item. Hand over a channel you own, or ask the owner to remove you.

Only the host removes someone from the workspace: see [Remove someone from the workspace](hosting-a-workspace.md#remove-someone-from-the-workspace). Ask the person to hand over their channels first, because a removed person stays their owner.

### Changes that end agent access

Adding or removing people, archiving a channel and handing one over end every chat's and agent task's access in the workspace, but renaming does not. See [Why settings changes end access](agents-and-chat-access.md#why-settings-changes-end-access) for the full list and how to grant access again.

## Names and avatars

### Set your display name

Others see your display name beside your messages, or your username, such as `@crew_alice`, until you set one. Your username is your lab server account. It identifies you and cannot be changed in Crew.

1. Choose the You row, then **Edit profile…**, which is available once your connection is verified.
2. Type your name in **Display name**. A name "Filled in from your account on {server}" is not used until you save.
3. Optionally, type up to 12 characters in **Initials (optional)**.
4. Choose **Save profile**. Your name appears beside your messages.

A display name has 1 to 64 characters, including a letter or number, and no `@` or `#`. It cannot be another member's username. It need not be unique, and it gives no rights. To show only your username, save it as your display name.

### How Crew shows a person

Crew shows a person as "Bob Lee (@bob)", or `@bob` without a display name. When two display names look alike, the username always shows. An agent appears as "Bob Lee's agent" or "Your agent", someone who left as "Bob Lee · former member", and an unknown person as "Unknown member".

People have round avatars, and agents square ones with a robot. An avatar shows your initials, or else the first letter of your display name or of your username's last part (`crew_alice` shows "A"). Its color comes from your username, so a display name cannot copy it.

## Less common situations

- With an older Crew on the server, only owners add people, and adding sends an invitation. The person chooses **Join** under "Invitations" in the sidebar, and the owner sees "{n} invited" beside the team until then.
- If "Available once the connection is verified" stays on a menu item, see [Connections and troubleshooting](connections-and-troubleshooting.md).

## Related documentation

- [Getting started](getting-started.md): the Crew window.
- [Hosting a workspace](hosting-a-workspace.md): inviting, letting in and removing people.
- [Joining a workspace](joining-a-workspace.md): what a new member sees.
- [Messages and files](messages-and-files.md): messages and files.
- [Agents and chat access](agents-and-chat-access.md): granting access again.
- [Privacy and security](privacy-and-security.md): Restricted and Public-safe channels.
- [Connections and troubleshooting](connections-and-troubleshooting.md): connection problems.
- [Command line](command-line.md): `biorouter crew` commands for every task on this page.
- [Administration](administration.md): workspace limits.
