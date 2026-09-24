import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest';
import { avatarInitials } from '../../ui/avatar';
import { PersonName } from './PersonName';
import { identityCopy } from './copy';
import {
  agentLabel,
  displayNameRepeatsUsername,
  joinerPerson,
  personFromProjection,
  personLabel,
  personRoles,
} from './personLabel';
import { buildPeopleDirectory } from './usePeopleDirectory';
import {
  PERSON_CONTEXTS,
  type CrewPrincipalInput,
  type DaemonPersonLabels,
  type PeopleSnapshotInput,
} from './types';

/**
 * The display rule, context by context (ui-redesign-spec, "Identity and naming
 * display rules"), and the one property every part of it shares: no rendering
 * and no string ever carries a principal ID, a 64-hex value or a numeric UID.
 */

const ID = {
  alice: '11111111-1111-4111-8111-111111111111',
  bob: '22222222-2222-4222-8222-222222222222',
  carol: '33333333-3333-4333-8333-333333333333',
  spark: '44444444-4444-4444-8444-444444444444',
  sampark: '55555555-5555-4555-8555-555555555555',
  dan: '66666666-6666-4666-8666-666666666666',
  sara: '77777777-7777-4777-8777-777777777777',
  ghost: '99999999-9999-4999-8999-999999999999',
};
const HEX64 = 'ab'.repeat(32);

const principal = (
  id: string,
  username: string,
  nickname: string,
  uid: number,
  extra: Partial<CrewPrincipalInput> = {}
): CrewPrincipalInput => ({ id, username, nickname, uid, ...extra });

const alice = principal(ID.alice, 'alice', 'Alice Chen', 1000);

function snapshot(overrides: Partial<PeopleSnapshotInput> = {}): PeopleSnapshotInput {
  return {
    workspace: { host_uid: 1000 },
    actor: alice,
    principals: [
      alice,
      principal(ID.bob, 'bob', 'Bob Lee', 1001),
      // A display name equal to the username, differing only in case.
      principal(ID.carol, 'carol', 'Carol', 1002),
      principal(ID.spark, 'spark', 'Sam Park', 1003),
      principal(ID.sampark, 'sampark', 'Sam  park', 1004),
      principal(ID.sara, 'sara', 'سارة', 1005),
    ],
    former_principals: [{ id: ID.dan, username: 'dan', display_name: 'Dan Wu', active: false }],
    ...overrides,
  };
}

const dir = buildPeopleDirectory(snapshot());

/** Every shape a machine reference in this fixture takes. */
const MACHINE_REFERENCE =
  /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|[0-9a-f]{32,}|\b10\d\d\b/i;

beforeAll(() => {
  // Radix Tooltip measures its trigger.
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});

afterAll(() => {
  vi.unstubAllGlobals();
});

const root = (container: HTMLElement) =>
  container.querySelector('[data-person-context]') as HTMLElement;

describe('PersonName, header', () => {
  it('shows the display name, then @username as its own muted element', () => {
    const { container } = render(<PersonName person={ID.bob} dir={dir} context="header" />);
    const name = screen.getByText('Bob Lee');
    expect(name.tagName).toBe('BDI');
    expect(name.closest('[data-person-part="name"]')).toHaveClass('text-label');
    const handle = screen.getByText('@bob');
    expect(handle.tagName).toBe('BDI');
    expect(handle).toHaveClass('text-supporting', 'text-text-muted');
    expect(handle.contains(name)).toBe(false);
    expect(name.contains(handle)).toBe(false);
    expect(root(container)).toHaveTextContent('Bob Lee @bob');
  });

  it('shows @username alone when the display name equals it case-insensitively', () => {
    const { container } = render(<PersonName person={ID.carol} dir={dir} context="header" />);
    expect(root(container)).toHaveTextContent(/^@carol$/);
    expect(screen.getByText('@carol').closest('[data-person-part="name"]')).toHaveClass(
      'text-label'
    );
    expect(screen.queryByText('Carol')).toBeNull();
  });

  it('names an agent after its owner, and the viewer’s own as "Your agent"', () => {
    const { container, rerender } = render(
      <PersonName person={ID.bob} dir={dir} context="header" agent />
    );
    expect(root(container)).toHaveTextContent("Bob Lee's agent @bob");
    rerender(<PersonName person={ID.alice} dir={dir} context="header" agent you />);
    expect(root(container)).toHaveTextContent('Your agent @alice');
  });
});

describe('PersonName, inline', () => {
  it('reads "Display name (@username)", with @username its own element', () => {
    const { container } = render(<PersonName person={ID.bob} dir={dir} context="inline" />);
    expect(root(container)).toHaveTextContent('Bob Lee (@bob)');
    expect(screen.getByText('@bob').tagName).toBe('BDI');
    expect(screen.getByText('Bob Lee').tagName).toBe('BDI');
  });

  it('reads "@username" when the names are equal', () => {
    const { container } = render(<PersonName person={ID.carol} dir={dir} context="inline" />);
    expect(root(container)).toHaveTextContent(/^@carol$/);
  });
});

describe('PersonName, authority', () => {
  it('shows both names, and @username once when the names are equal', () => {
    const { container, rerender } = render(
      <PersonName person={ID.bob} dir={dir} context="authority" />
    );
    expect(root(container)).toHaveTextContent(/^Bob Lee \(@bob\)$/);
    rerender(<PersonName person={ID.carol} dir={dir} context="authority" />);
    expect(root(container)).toHaveTextContent(/^@carol$/);
    expect(screen.getByText('@carol')).toHaveAttribute('data-person-part', 'username');
    expect(screen.queryByText('Carol')).toBeNull();
  });

  /**
   * What an authority point has to show in full is the one name nobody can
   * choose to look like someone else's: `@username`. Dropping a display name
   * that only repeats it (T-31) must never drop the handle itself.
   */
  it('always shows the full @username, for every person and option', () => {
    for (const id of [ID.alice, ID.bob, ID.carol, ID.spark, ID.sampark, ID.sara, ID.dan]) {
      const username = dir.byId(id)!.username;
      for (const options of [{}, { agent: true }, { you: true }, { agent: true, you: true }]) {
        const { container, unmount } = render(
          <PersonName person={id} dir={dir} context="authority" {...options} />
        );
        const handle = container.querySelector('[data-person-part="username"]');
        expect(handle).toHaveTextContent(new RegExp(`^@${username}$`));
        expect(personLabel(id, 'authority', dir, options)).toContain(`@${username}`);
        unmount();
      }
    }
  });

  it('names the viewer’s agent in full at an authority point', () => {
    const { container } = render(
      <PersonName person={ID.alice} dir={dir} context="authority" agent you />
    );
    expect(root(container)).toHaveTextContent("Alice Chen (@alice)'s agent");
  });
});

describe('PersonName, chip', () => {
  it('shows the display name, with @username in a tooltip and in text for assistive technology', async () => {
    const user = userEvent.setup();
    const { container } = render(<PersonName person={ID.bob} dir={dir} context="chip" />);
    const chip = root(container);
    expect(screen.getByText('Bob Lee').tagName).toBe('BDI');
    const hidden = chip.querySelector('.sr-only');
    expect(hidden).toHaveTextContent('(@bob)');
    expect(hidden?.querySelector('[data-person-part="username"]')).toHaveTextContent('@bob');

    await user.hover(chip);
    expect(await screen.findByRole('tooltip')).toHaveTextContent('@bob');
  });

  it('spells out "Display name (@username)" on a collision, with no tooltip needed', () => {
    const { container, rerender } = render(
      <PersonName person={ID.spark} dir={dir} context="chip" />
    );
    expect(root(container)).toHaveTextContent('Sam Park (@spark)');
    expect(root(container).querySelector('.sr-only')).toBeNull();
    rerender(<PersonName person={ID.sampark} dir={dir} context="chip" />);
    expect(root(container)).toHaveTextContent('Sam park (@sampark)');
  });

  it('shows @username alone when the names are equal', () => {
    const { container } = render(<PersonName person={ID.carol} dir={dir} context="chip" />);
    expect(root(container)).toHaveTextContent(/^@carol$/);
  });

  it('can leave the tooltip to a surrounding one', () => {
    render(<PersonName person={ID.bob} dir={dir} context="chip" tooltip={false} />);
    expect(screen.getByText('Bob Lee')).toBeInTheDocument();
    expect(screen.getByText('@bob').closest('.sr-only')).not.toBeNull();
  });
});

describe('PersonName, joiner at a host decision', () => {
  it('shows @username first in mono, then the name on the server account', () => {
    const { container } = render(
      <PersonName person={joinerPerson('bob', 'Bob Lee')} context="joiner" />
    );
    const handle = screen.getByText('@bob');
    expect(handle).toHaveClass('font-mono');
    expect(root(container)).toHaveTextContent('@bob · Bob Lee (name on the server account)');
    expect(root(container).firstElementChild).toBe(handle);
    expect(screen.getByText('Bob Lee').tagName).toBe('BDI');
  });

  it('shows only @username when the server account carries no name', () => {
    const { container } = render(<PersonName person={joinerPerson('bob')} context="joiner" />);
    expect(root(container)).toHaveTextContent(/^@bob$/);
  });
});

describe('former members', () => {
  it('render muted with " · former member" in every context', () => {
    for (const context of ['header', 'inline', 'authority', 'chip'] as const) {
      const { container, unmount } = render(
        <PersonName person={ID.dan} dir={dir} context={context} tooltip={false} />
      );
      expect(root(container)).toHaveClass('text-text-muted');
      expect(root(container)).toHaveAttribute('data-person-state', 'former');
      expect(screen.getByText(identityCopy.formerMember)).toBeInTheDocument();
      expect(root(container).textContent).toMatch(/· former member$/);
      unmount();
    }
  });

  it('read the same way as strings', () => {
    expect(personLabel(ID.dan, 'authority', dir)).toBe('Dan Wu (@dan) · former member');
    expect(personLabel(ID.dan, 'inline', dir)).toBe('Dan Wu (@dan) · former member');
  });
});

describe('unknown members', () => {
  it('render "Unknown member" and never the ID they were asked about', () => {
    for (const context of PERSON_CONTEXTS) {
      const { container, unmount } = render(
        <PersonName person={ID.ghost} dir={dir} context={context} />
      );
      expect(root(container)).toHaveTextContent(/^Unknown member$/);
      expect(container.innerHTML).not.toContain(ID.ghost);
      expect(personLabel(ID.ghost, context, dir)).toBe('Unknown member');
      unmount();
    }
  });

  it('treat an ID with no directory, a missing person and a malformed one alike', () => {
    expect(personLabel(ID.bob, 'inline')).toBe('Unknown member');
    expect(personLabel(null, 'inline', dir)).toBe('Unknown member');
    expect(personLabel(undefined, 'header', dir)).toBe('Unknown member');
    expect(personLabel(joinerPerson('   '), 'joiner')).toBe('Unknown member');
    expect(agentLabel(ID.ghost, 'inline', dir)).toBe("Unknown member's agent");
  });
});

describe('isolation', () => {
  it('wraps every display name in its own <bdi>, apart from @username', () => {
    const { container } = render(<PersonName person={ID.sara} dir={dir} context="inline" />);
    const name = screen.getByText('سارة');
    expect(name.tagName).toBe('BDI');
    expect(name.textContent).toBe('سارة');
    const handle = container.querySelector('[data-person-part="username"]');
    expect(handle?.tagName).toBe('BDI');
    expect(handle?.textContent).toBe('@sara');
  });

  it('wraps a right-to-left name in Unicode isolates inside a string, and leaves others plain', () => {
    expect(personLabel(ID.sara, 'authority', dir)).toBe('\u2068سارة\u2069 (@sara)');
    expect(personLabel(ID.bob, 'authority', dir)).toBe('Bob Lee (@bob)');
  });

  it('strips bidi overrides and zero-width characters from a legacy nickname', () => {
    const hostile = buildPeopleDirectory(
      snapshot({
        principals: [alice, principal(ID.bob, 'bob', 'Bob\u202E Lee\u200B\u2066', 1001)],
      })
    );
    expect(personLabel(ID.bob, 'inline', hostile)).toBe('Bob Lee (@bob)');
  });
});

describe('the four contexts as strings', () => {
  it.each([
    ['header', 'Bob Lee (@bob)', '@carol'],
    ['inline', 'Bob Lee (@bob)', '@carol'],
    ['authority', 'Bob Lee (@bob)', '@carol'],
    ['chip', 'Bob Lee', '@carol'],
  ] as const)('%s', (context, bob, carol) => {
    expect(personLabel(ID.bob, context, dir)).toBe(bob);
    expect(personLabel(ID.carol, context, dir)).toBe(carol);
    expect(personLabel(ID.spark, context, dir)).toBe('Sam Park (@spark)');
  });

  it('labels agents per the rule', () => {
    expect(agentLabel(ID.bob, 'inline', dir)).toBe("Bob Lee's agent");
    expect(agentLabel(ID.bob, 'chip', dir)).toBe("Bob Lee's agent");
    expect(agentLabel(ID.bob, 'header', dir)).toBe("Bob Lee's agent (@bob)");
    expect(agentLabel(ID.bob, 'authority', dir)).toBe("Bob Lee (@bob)'s agent");
    expect(agentLabel(ID.spark, 'inline', dir)).toBe("Sam Park (@spark)'s agent");
    expect(agentLabel(ID.carol, 'inline', dir)).toBe("@carol's agent");
    expect(agentLabel(ID.carol, 'authority', dir)).toBe("@carol's agent");
    expect(agentLabel(ID.alice, 'inline', dir, { you: true })).toBe('Your agent');
    expect(agentLabel(ID.alice, 'authority', dir, { you: true })).toBe(
      "Alice Chen (@alice)'s agent"
    );
    expect(agentLabel(ID.dan, 'inline', dir)).toBe("Dan Wu's agent · former member");
  });

  it('marks the viewer only when asked', () => {
    expect(personLabel(ID.alice, 'inline', dir)).toBe('Alice Chen (@alice)');
    expect(personLabel(ID.alice, 'inline', dir, { you: true })).toBe('Alice Chen (@alice) · you');
  });

  it('labels a joiner', () => {
    expect(personLabel(joinerPerson('bob', 'Bob Lee'), 'joiner')).toBe(
      '@bob · Bob Lee (name on the server account)'
    );
  });
});

describe('a person with no display name of their own (T-31)', () => {
  /**
   * The round-1 stage: server accounts issued as `crew_…`, nobody has set a
   * display name yet, so each nickname is the username. Every member row and
   * access line read `crew_alice (@crew_alice)`.
   */
  const unnamed = buildPeopleDirectory(
    snapshot({
      actor: principal(ID.alice, 'crew_alice', 'crew_alice', 1000),
      principals: [
        principal(ID.alice, 'crew_alice', 'crew_alice', 1000),
        principal(ID.bob, 'crew_bob', 'Crew_Bob', 1001),
        principal(ID.carol, 'crew_carol', '', 1002),
      ],
      former_principals: [
        { id: ID.dan, username: 'crew_dan', display_name: 'crew_dan', active: false },
      ],
    })
  );
  const DOUBLED = /(@?)(crew_[a-z]+) \(@\2\)/i;

  it('reads @username once in every context, as a string and rendered', () => {
    for (const [id, username] of [
      [ID.alice, 'crew_alice'],
      [ID.bob, 'crew_bob'],
      [ID.carol, 'crew_carol'],
    ] as const) {
      for (const context of ['header', 'inline', 'authority', 'chip'] as const) {
        expect(personLabel(id, context, unnamed)).toBe(`@${username}`);
        const { container, unmount } = render(
          <PersonName person={id} dir={unnamed} context={context} />
        );
        expect(root(container)).toHaveTextContent(new RegExp(`^@${username}$`));
        expect(container.querySelectorAll('[data-person-part="username"]')).toHaveLength(1);
        expect(container.querySelector('[data-person-part="display-name"]')).toBeNull();
        unmount();
      }
    }
  });

  it('never doubles the name for an agent, the viewer or a former member', () => {
    for (const context of ['header', 'inline', 'authority', 'chip'] as const) {
      for (const options of [{ agent: true }, { you: true }, { agent: true, you: true }]) {
        for (const id of [ID.alice, ID.bob, ID.dan]) {
          expect(personLabel(id, context, unnamed, options)).not.toMatch(DOUBLED);
          const { container, unmount } = render(
            <PersonName person={id} dir={unnamed} context={context} {...options} />
          );
          expect(container.textContent).not.toMatch(DOUBLED);
          unmount();
        }
      }
    }
    expect(personLabel(ID.alice, 'authority', unnamed, { you: true })).toBe('@crew_alice · you');
    expect(agentLabel(ID.alice, 'authority', unnamed, { you: true })).toBe("@crew_alice's agent");
    expect(personLabel(ID.dan, 'authority', unnamed)).toBe('@crew_dan · former member');
  });

  it('still spells out a display name the person chose', () => {
    const named = buildPeopleDirectory(
      snapshot({
        principals: [principal(ID.alice, 'crew_alice', 'Alice Chen', 1000)],
        actor: principal(ID.alice, 'crew_alice', 'Alice Chen', 1000),
      })
    );
    expect(personLabel(ID.alice, 'authority', named)).toBe('Alice Chen (@crew_alice)');
    expect(personLabel(ID.alice, 'inline', named)).toBe('Alice Chen (@crew_alice)');
  });
});

describe('a fully qualified SSSD account with no display name of its own (T-31)', () => {
  /**
   * `valid_username` admits `bob@ad.ucsf.edu`, and a new principal's nickname
   * is its username. The daemon's `sanitize_display_name` removes the `@`, so it
   * projects `display_name: "bobad.ucsf.edu"` — and a legacy daemon forwards
   * the nickname as stored, which `usableName` strips the same way. Either way
   * the person chose nothing, so they read `@bob@ad.ucsf.edu` once, and their
   * avatar is read from the account part, never from the shared realm.
   */
  const SSSD = [
    ['bob@ad.ucsf.edu', 'B'],
    ['alice@ad.ucsf.edu', 'A'],
    ['carol@ad.ucsf.edu', 'C'],
    ['crew_bob@ad.ucsf.edu', 'B'],
  ] as const;

  it('reads @username once and the account part’s initial, off a daemon projection', () => {
    for (const [username, initial] of SSSD) {
      for (const display_name of [username.replace('@', ''), username, undefined]) {
        const person = personFromProjection({ username, display_name, nickname: username });
        expect(person).not.toBeNull();
        expect(displayNameRepeatsUsername(person!.displayName, person!.username)).toBe(true);
        for (const context of ['header', 'inline', 'authority', 'chip'] as const) {
          expect(personLabel(person, context)).toBe(`@${username}`);
        }
        expect(agentLabel(person, 'authority')).toBe(`@${username}'s agent`);
        expect(avatarInitials(person!.displayName, person!.username)).toBe(initial);
      }
    }
  });

  it('renders the handle alone, in a directory too', () => {
    const sssd = buildPeopleDirectory(
      snapshot({
        actor: principal(ID.alice, 'alice@ad.ucsf.edu', 'alicead.ucsf.edu', 1000),
        principals: [
          principal(ID.alice, 'alice@ad.ucsf.edu', 'alicead.ucsf.edu', 1000),
          // A legacy nickname, stored before the daemon stripped it.
          principal(ID.bob, 'bob@ad.ucsf.edu', 'bob@ad.ucsf.edu', 1001),
          principal(ID.carol, 'carol@ad.ucsf.edu', 'Carol Nguyen', 1002),
        ],
        former_principals: [],
      })
    );
    for (const [id, username] of [
      [ID.alice, 'alice@ad.ucsf.edu'],
      [ID.bob, 'bob@ad.ucsf.edu'],
    ] as const) {
      for (const context of ['header', 'inline', 'authority', 'chip'] as const) {
        expect(personLabel(id, context, sssd)).toBe(`@${username}`);
        const { container, unmount } = render(
          <PersonName person={id} dir={sssd} context={context} />
        );
        expect(root(container)).toHaveTextContent(new RegExp(`^@${username}$`));
        expect(container.querySelector('[data-person-part="display-name"]')).toBeNull();
        unmount();
      }
    }
    // A name the person chose is still spelled out beside the full handle.
    expect(personLabel(ID.carol, 'authority', sssd)).toBe('Carol Nguyen (@carol@ad.ucsf.edu)');
    expect(avatarInitials(sssd.byId(ID.carol)!.displayName, 'carol@ad.ucsf.edu')).toBe('CN');
  });

  it('counts only the username’s own shapes as a repeat', () => {
    expect(displayNameRepeatsUsername('BOBad.ucsf.edu', 'bob@ad.ucsf.edu')).toBe(true);
    expect(displayNameRepeatsUsername('crew_alice', 'crew_alice')).toBe(true);
    expect(displayNameRepeatsUsername('Bob', 'bob@ad.ucsf.edu')).toBe(false);
    expect(displayNameRepeatsUsername('bob ad ucsf edu', 'bob@ad.ucsf.edu')).toBe(false);
    expect(displayNameRepeatsUsername('alicead.ucsf.edu', 'bob@ad.ucsf.edu')).toBe(false);
  });
});

describe('role names', () => {
  it('names the workspace role "Host" and a channel\u2019s role "Owner", and never one for the other', () => {
    expect(personRoles).toEqual({ host: 'Host', owner: 'Owner' });
    expect(personRoles.host).not.toBe(personRoles.owner);
    expect(Object.isFrozen(personRoles)).toBe(true);
  });
});

describe('collisions', () => {
  it('find two people named "Sam Park" by name key when the daemon projects no labels', () => {
    expect(dir.collides(ID.spark)).toBe(true);
    expect(dir.collides(ID.sampark)).toBe(true);
    expect(dir.collides(ID.bob)).toBe(false);
  });

  it('take the daemon’s projected verdict, which also covers confusable skeletons', () => {
    const labels: DaemonPersonLabels = {
      [ID.bob]: { full: 'Bob Lee (@bob)', short: 'Bob Lee', collides: true },
      [ID.spark]: { full: 'Sam Park (@spark)', short: 'Sam Park', collides: false },
    };
    const projected = buildPeopleDirectory(snapshot(), labels);
    expect(personLabel(ID.bob, 'chip', projected)).toBe('Bob Lee (@bob)');
    expect(personLabel(ID.spark, 'chip', projected)).toBe('Sam Park');
    // No label for this person: the local rule still applies.
    expect(personLabel(ID.sampark, 'chip', projected)).toBe('Sam park (@sampark)');
  });

  it('refresh a person object from the directory, so a stale copy cannot hide a collision', () => {
    const stale = { ...dir.byId(ID.spark)!, collides: false };
    expect(personLabel(stale, 'chip', dir)).toBe('Sam Park (@spark)');
    expect(personLabel(stale, 'chip')).toBe('Sam Park');
  });
});

describe('never an ID', () => {
  /**
   * Hostile and legacy nicknames: an ID in each shape, the actor's own numeric
   * UID, a name that imitates another person's username, and a forged handle.
   */
  const hostile = buildPeopleDirectory(
    snapshot({
      principals: [
        alice,
        principal(ID.bob, 'bob', ID.carol, 1001),
        principal(ID.carol, 'carol', HEX64, 1002),
        principal(ID.spark, 'spark', '1003', 1003),
        principal(ID.sampark, 'sampark', 'alice', 1004),
        principal(ID.sara, 'sara', 'Alice (@alice)', 1005),
      ],
    }),
    { [ID.alice]: { full: ID.alice, short: HEX64, collides: false } },
    { [ID.ghost]: { username: 'eve', display_name: `{${ID.ghost}}`, active: false } }
  );

  it('falls back to the username for a nickname that reads as a machine reference', () => {
    expect(personLabel(ID.bob, 'chip', hostile)).toBe('@bob');
    expect(personLabel(ID.carol, 'chip', hostile)).toBe('@carol');
    expect(personLabel(ID.spark, 'chip', hostile)).toBe('@spark');
    expect(personLabel(ID.ghost, 'chip', hostile)).toBe('@eve · former member');
  });

  it('drops a server-account name that reads as a machine reference', () => {
    expect(personLabel(joinerPerson('bob', ID.bob), 'joiner')).toBe('@bob');
    expect(personLabel(joinerPerson('bob', '1001'), 'joiner')).toBe('@bob');
  });

  it('does not let a nickname pass for someone else’s username or carry a second handle', () => {
    expect(personLabel(ID.sampark, 'chip', hostile)).toBe('@sampark');
    expect(personLabel(ID.sara, 'inline', hostile)).toBe('Alice (alice) (@sara)');
  });

  it('holds for every person, context and option, as strings and as rendered text', () => {
    const people = [...Object.values(ID), joinerPerson('bob', ID.bob), null];
    for (const directory of [dir, hostile]) {
      for (const person of people) {
        for (const context of PERSON_CONTEXTS) {
          for (const options of [{}, { agent: true }, { you: true }, { agent: true, you: true }]) {
            expect(personLabel(person, context, directory, options)).not.toMatch(MACHINE_REFERENCE);
            const { container, unmount } = render(
              <PersonName person={person} dir={directory} context={context} {...options} />
            );
            expect(container.textContent).not.toMatch(MACHINE_REFERENCE);
            expect(container.innerHTML).not.toMatch(
              /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i
            );
            unmount();
          }
        }
      }
    }
  });
});

describe('people directory', () => {
  it('covers active principals, the actor, former principals and a message result’s people map', () => {
    const directory = buildPeopleDirectory(
      snapshot({ principals: [principal(ID.bob, 'bob', 'Bob Lee', 1001)] }),
      null,
      { [ID.ghost]: { username: 'eve', display_name: 'Eve Ng', active: false } }
    );
    expect(directory.byId(ID.alice)?.displayName).toBe('Alice Chen');
    expect(directory.byId(ID.bob)?.displayName).toBe('Bob Lee');
    expect(directory.isFormer(ID.dan)).toBe(true);
    expect(personLabel(ID.ghost, 'inline', directory)).toBe('Eve Ng (@eve) · former member');
    expect(directory.people.map((p) => p.username)).toEqual(['bob', 'alice']);
    expect(directory.formerPeople.map((p) => p.username)).toEqual(['dan', 'eve']);
    expect(directory.me?.username).toBe('alice');
    expect(directory.byId(ID.alice)?.isYou).toBe(true);
    expect(directory.byId(null)).toBeNull();
  });

  it('prefers the projected display name to the nickname', () => {
    const directory = buildPeopleDirectory(
      snapshot({
        principals: [
          alice,
          { ...principal(ID.bob, 'bob', 'bobby', 1001), display_name: 'Bob Lee' },
        ],
      })
    );
    expect(directory.byId(ID.bob)?.displayName).toBe('Bob Lee');
  });

  it('detects the host by host_principal_id, else by the host UID', () => {
    const byUid = buildPeopleDirectory(snapshot());
    expect(byUid.isHost(ID.alice)).toBe(true);
    expect(byUid.host?.username).toBe('alice');
    expect(byUid.viewerIsHost).toBe(true);

    const byPrincipal = buildPeopleDirectory(
      snapshot({ workspace: { host_uid: 1000, host_principal_id: ID.bob } })
    );
    expect(byPrincipal.isHost(ID.alice)).toBe(false);
    expect(byPrincipal.isHost(ID.bob)).toBe(true);
    expect(byPrincipal.viewerIsHost).toBe(false);

    const bobViewing = buildPeopleDirectory(
      snapshot({ actor: principal(ID.bob, 'bob', 'Bob Lee', 1001) })
    );
    expect(bobViewing.viewerIsHost).toBe(false);
    expect(bobViewing.host?.username).toBe('alice');
  });

  it('is empty without a snapshot, and drops malformed principals rather than echoing them', () => {
    const empty = buildPeopleDirectory(null);
    expect(empty.people).toEqual([]);
    expect(empty.me).toBeNull();
    expect(empty.viewerIsHost).toBe(false);

    const malformed = buildPeopleDirectory({
      workspace: { host_uid: 1000 },
      actor: alice,
      principals: [
        alice,
        { id: ID.bob, username: '' } as CrewPrincipalInput,
        { id: '', username: 'nobody' } as CrewPrincipalInput,
        null as unknown as CrewPrincipalInput,
      ],
    });
    expect(malformed.people.map((p) => p.username)).toEqual(['alice']);
    expect(personLabel(ID.bob, 'inline', malformed)).toBe('Unknown member');
  });
});

describe('a projection outside the directory', () => {
  it('renders an invitation’s projected inviter', () => {
    const inviter = personFromProjection({ username: 'alice', display_name: 'Alice Chen' });
    expect(personLabel(inviter, 'inline')).toBe('Alice Chen (@alice)');
    expect(personFromProjection({ username: '' })).toBeNull();
    expect(personLabel(personFromProjection(null), 'inline')).toBe('Unknown member');
  });
});
