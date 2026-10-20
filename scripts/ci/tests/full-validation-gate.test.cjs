'use strict';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const { requireFullValidation } = require('../../../.github/scripts/full-validation-gate.cjs');
const sha = 'a'.repeat(40);
const names = ['test', 'test_macos', 'coverage', 'test_containment', 'cargo_deny'];
function fixture() {
  return Object.fromEntries(names.map(name => [name, { result: 'success' }]));
}
test('full exact-commit validation admits publication', () => {
  assert.equal(requireFullValidation({ expected: sha, actual: sha, needs: fixture() }), true);
});
for (const name of names) {
  for (const result of ['failure', 'cancelled', 'skipped', undefined]) {
    test(`${name}/${result} blocks asset publication and tap dispatch`, () => {
      const needs = fixture();
      needs[name] = { result };
      const writes = [];
      assert.throws(() => {
        requireFullValidation({ expected: sha, actual: sha, needs });
        writes.push('assets', 'tap');
      }, /full validation/i);
      assert.deepEqual(writes, []);
    });
  }
}
test('a different or missing commit fails closed', () => {
  for (const actual of ['b'.repeat(40), '', undefined]) {
    assert.throws(() => requireFullValidation({ expected: sha, actual, needs: fixture() }), /commit/i);
  }
});
