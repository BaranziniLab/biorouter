import { describe, expect, it } from 'vitest';
import { KNOWN_FN_APPS, collidingFnApp, listedFnApps, type KnownFnApp } from './fnCollision';

describe('the apps known to use fn', () => {
  it('lists Wispr Flow by its bundle id, with the name Biorouter shows', () => {
    expect(KNOWN_FN_APPS).toEqual([{ bundle: 'com.electron.wispr-flow', name: 'Wispr Flow' }]);
    // The table is read only: nothing at run time can add an app or rename one.
    expect(Object.isFrozen(KNOWN_FN_APPS)).toBe(true);
    for (const app of KNOWN_FN_APPS) expect(Object.isFrozen(app)).toBe(true);
  });

  it('finds a listed app among the bundles that watch modifier keys', () => {
    expect(collidingFnApp([])).toBeNull();
    expect(collidingFnApp(['com.example.hotkeys', 'com.electron.wispr-flow'])).toEqual({
      bundle: 'com.electron.wispr-flow',
      name: 'Wispr Flow',
    });
    // The answer is the table's own entry, never text another program wrote.
    expect(collidingFnApp(['com.electron.wispr-flow'])).toBe(KNOWN_FN_APPS[0]);
  });

  it('matches bundle ids exactly, never by prefix, case or spacing', () => {
    for (const near of [
      'com.electron.wispr-flow.helper',
      'com.electron.wispr',
      'COM.ELECTRON.WISPR-FLOW',
      'Com.Electron.Wispr-Flow',
      ' com.electron.wispr-flow',
      'com.electron.wispr-flow ',
      'xcom.electron.wispr-flow',
      'Wispr Flow',
    ]) {
      expect(collidingFnApp([near]), near).toBeNull();
    }
  });

  it('an app that is not listed is never a collision, whatever it is called', () => {
    expect(collidingFnApp(['com.logi.cp-dev-mgr', 'com.example.fn-dictation', 'fn'])).toBeNull();
  });

  it('answers in table order, not in the order the bundles were reported', () => {
    const table: readonly KnownFnApp[] = [
      { bundle: 'com.example.first', name: 'First' },
      { bundle: 'com.example.second', name: 'Second' },
    ];
    expect(collidingFnApp(['com.example.second', 'com.example.first'], table)).toBe(table[0]);
    expect(collidingFnApp(['com.example.second'], table)).toBe(table[1]);
    expect(collidingFnApp(['com.example.third'], table)).toBeNull();
  });

  it('lists every listed app present, in table order, for the helper restart', () => {
    const table: readonly KnownFnApp[] = [
      { bundle: 'com.example.first', name: 'First' },
      { bundle: 'com.example.second', name: 'Second' },
    ];
    expect(listedFnApps(['com.example.second', 'x', 'com.example.first'], table)).toEqual(table);
    expect(listedFnApps(['com.example.second'], table)).toEqual([table[1]]);
    expect(listedFnApps([], table)).toEqual([]);
    expect(listedFnApps(['com.electron.wispr-flow'])).toEqual([KNOWN_FN_APPS[0]]);
  });
});
