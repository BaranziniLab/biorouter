# Biorouter Technical Governance and Stewardship

Learn about Biorouter's governance structure and how to participate

Biorouter follows a lightweight technical governance model designed to support rapid iteration while maintaining community involvement. This document outlines how the project is organized and how decisions are made.

> [!IMPORTANT]
> **Status: target-state model; the roles below are not yet fully populated.**
> Biorouter is today maintained by a single Core Maintainer — see
> [MAINTAINERS.md](MAINTAINERS.md) for the current membership. The group is below
> the three-to-seven size this document aims for, so every rule below that calls
> for a *majority vote of Core Maintainers* describes how decisions will be made
> once the group is grown, not how they are made today. Until then, decisions rest
> with the Core Maintainer listed in MAINTAINERS.md, taken in the open on GitHub.
> Read this document as the model the project is committing to, and treat any gap
> between it and MAINTAINERS.md as a gap to be closed by appointing maintainers,
> not as a description of a body that already exists.

## Core Values
Biorouter's governance is guided by three fundamental values:

* **Open**: Biorouter is open source, but we go beyond code availability. We plan and build in the open. Our roadmap as well as Biorouter recipes, extensions, and prompts are editable and shareable. Our goal is to make Biorouter the most hackable agent available.
* **Flexible**: we prefer open models – but we don't restrict ourselves. Biorouter equally supports remotely deployed frontier models as well as local private models, whether open or proprietary.
* **Choice**: We're not bound to any one model, protocol, or stack. Biorouter is built for choice and open standards, adapting to your tools, workflow, and identity as a creator.

## Roles

### Contributors

Anyone in the community who contributes to Biorouter through issues, pull requests, or discussions. Community contributions of all kinds, from code and bug reports to feature requests and discussion participation, help ensure Biorouter evolves in directions that serve real user needs and remains aligned with how people actually use the project.

### Maintainers

Maintainers are trusted community members responsible for key components of Biorouter. They review pull requests, guide contributors, and ensure technical and community health within their domain.

#### Responsibilities

* Drive ongoing improvements within their component area.
* Review contributions and maintain quality.
* Foster community participation.
* Surface strategic or architectural decisions to Core Maintainers when needed.

Maintainers have write access to create branches on the repository but not full administrative rights.

### Core Maintainers

Core Maintainers have broad technical understanding of Biorouter and are responsible for the project's overall direction, technical consistency, and long-term vision.

#### Responsibilities

* Setting the overall technical direction and vision for Biorouter
* Define and uphold Biorouter's technical direction and principles.
* Resolve disputes escalated by Maintainers.
* Appoint and remove Maintainers.
* Ensure the balance between innovation and stability.
* Steward Biorouter in the best interest of the open community.

Core Maintainers have admin access across all repositories but use standard contribution workflows (e.g., pull requests) for transparency.

## Decision-Making Process

### Day-to-Day Decisions

* Most technical and process decisions are made through consensus in issues and pull requests on GitHub. GitHub Discussions is not enabled on the repository; issues are the public venue for questions and proposals.
* Core Maintainers can approve and merge changes quickly when there's clear benefit.
* Significant architectural changes should be discussed in a GitHub issue before implementation.
* Core Maintainers may step in when disputes arise or when decisions have project-wide impact.

### Dispute Resolution

* If Maintainers cannot reach consensus, the matter is escalated to Core Maintainers.
* Core Maintainers aim for consensus through discussion.
* If no resolution is reached after reasonable discussion, Core Maintainers may hold a simple majority vote to resolve the issue.
* All dispute resolutions must be publicly announced on GitHub, which is the record of decision. Announcements may also be relayed to the UCSF Slack for lab members.

This process ensures fairness and transparency while enabling timely decision-making.

### Deadlocks

In the event of a decision deadlock in the process above, Biorouter's maintainer, Wanjun Gu, steps in as a tie breaker to remove the deadlock and make progress.

### Major Changes

Major architectural or directional changes should:

1. Be proposed as a GitHub issue.
2. Undergo open community review for at least one week.
3. Require approval from a majority of Core Maintainers.
4. Be publicly announced on GitHub.

## Selection and Removal of Maintainers

### Principles

* Membership is merit-based and tied to individual contributions, not employer affiliation.
* No term limits, but inactivity may lead to emeritus status.
* All appointments and removals are made transparently.

### Maintainer Nomination Process

1. Nomination by any existing Maintainer or Core Maintainer, based on:
   * Sustained, high-quality contributions.
   * Constructive participation in reviews and discussions.
   * Alignment with the project’s values.
2. Discussion among all Core Maintainers.
3. Approval by majority vote.
4. Public announcement on GitHub.

### Core Maintainer Appointment

We aim to have between 3 and 7 Core Maintainers at any time. We strive for an odd number of Core Maintainers to minimise the chances of voting deadlocks, but technical excellence of candidates takes precedence over adhering precisely to numbers.

1. Nomination by any existing Core Maintainer, based on:
   * Everything required to be a maintainer
   * Demonstrated leadership and judgement.
   * Long-term commitment to the project’s values
2. Discussion among all Core Maintainers.
3. Approval by majority vote.
4. Public announcement on GitHub.

### Removal

Maintainers or Core Maintainers may be removed in the following cases:

* Extended inactivity of 3+ months without contribution.
* Actions contrary to the project’s values.
* By their own request.

Removal decisions require a majority vote of Core Maintainers and must be documented publicly. Appeals can be made to the Core Maintainers with supporting rationale.

### Succession Planning

If a Core Maintainer leaves for any reason:

* The remaining Core Maintainers should appoint a replacement within 30 days.
* If Core Maintainer count falls below three, appointment of new Core Maintainers becomes a priority and appointment must happen within 15 days. No major decisions will be made until a new Core Maintainer is appointed.
* If there is no qualified developer available who is willing to serve as a Core Maintainer, the remaining Core Maintainer(s) shall instead create and adopt a plan for recruiting and mentoring a new Core Maintainer.
* Until a replacement is appointed, remaining Core Maintainers continue governance responsibilities

## Communication

### Channels

* **GitHub**: The canonical home for issues, pull requests, and documentation, and the only venue of record for decisions. GitHub Discussions is not enabled; use issues.
* **UCSF Slack**: An internal Baranzini Lab channel, not reachable by outside contributors and with no public join path. It is used for real-time collaboration and informal chat only. Nothing decided there counts until it is written down on GitHub, so an outside contributor loses no standing by not being in it.

### Transparency

* All technical decisions and governance discussions are to be conducted publicly.
* Meeting notes and key decisions are published openly on GitHub.
* Roadmap and priorities are openly discussed and published on GitHub.
* All proposals and changes to governance must be documented via pull requests.

## Working Practices

### Code Review

#### Following our way of working

* Review AI-generated work carefully: Check for redundant comments, tests that provide little to negative value, outdated patterns, and repeated code.
* Prioritize reviews: Others are waiting, but take time to understand the changes.
* Avoid review shopping: Seek review from those familiar with the code being modified.
* Test thoroughly: Manual and automated E2E testing is essential for larger features; post screenshots or videos for UI changes.

#### Contributing

* Discuss first: For new features or architectural changes, open an issue.
* Keep PRs focused: Smaller, focused changes are easier to review and merge.
* Write meaningful tests: Tests should guard against real bugs, not just increase coverage.
* Engage with the community: All Maintainers should be active on GitHub — the venue every contributor can reach — and be responsive to other contributors.

#### Release Process

* Regular releases with clear documentation of delivered features.
* Quick bug fixes or security resolutions are cherry-picked to patch releases when needed.
* Every release is tested before publication by the maintainer cutting it. Review by a second maintainer is the goal, but with the Core Maintainer group at its present size it is not what happens today — do not read this as a multi-party release gate.

## Governance Changes

This governance model may evolve as Biorouter grows. Any proposed modification to this document must:

1. Be proposed through a GitHub issue with rationale.
2. Undergo open community discussion for at least one week.
3. Be approved by a majority of Core Maintainers.
4. Clear communication of changes to the community.
5. Implemented via a pull request to the GOVERNANCE.md file in the main Biorouter repository.

## Current Membership

Core Maintainers and Maintainers are listed in the main Biorouter repository's [MAINTAINERS.md](https://github.com/BaranziniLab/biorouter/blob/main/MAINTAINERS.md) file with their areas of expertise where applicable.

## Summary

### Biorouter's governance prioritizes

* **Speed**: Minimal process to support rapid experimentation
* **Openness**: Transparent decision-making and community involvement
* **Autonomy**: Empowering users and contributors to shape Biorouter
* **Quality**: Thoughtful review while avoiding bureaucracy

We believe this balance enables Biorouter to remain innovative while building a strong, engaged community around the shared goal of creating the most hackable, user-controlled AI agent available.

# General Project Policies

Biorouter is developed and maintained by the [Baranzini Lab](https://baranzinilab.ucsf.edu/) at the University of California, San Francisco. It is an independent project, not a fork of Goose. Its design was strongly influenced by Block's open source [Goose](https://github.com/block/goose) agent, and it also draws on the other agents listed in the [README](README.md#credits-and-citation) and on many open source libraries.

Biorouter participants acknowledge that the copyright in all new contributions will be retained by the copyright holder as independent works of authorship and that no contributor or copyright holder will be required to assign copyrights to the project.
Except as described below, all code and specification contributions to the project must be made using the Apache License, Version 2.0, available at https://www.apache.org/licenses/LICENSE-2.0 (the “Project License”).

All outbound code and specifications will be made available under the Project License. The Core Maintainers may approve the use of an alternative open license or licenses for inbound or outbound contributions on an exception basis.
All documentation (excluding specifications) will be made available under the Creative Commons Attribution 4.0 International license, available at: https://creativecommons.org/licenses/by/4.0.
