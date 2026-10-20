'use strict';

const REQUIRED = Object.freeze(['test', 'test_macos', 'coverage', 'test_containment', 'cargo_deny']);

function requireFullValidation({ expected, actual, needs }) {
  if (!/^[0-9a-f]{40}$/.test(expected || '') || actual !== expected) {
    throw new Error('Full validation commit does not match the requested commit');
  }
  const failed = REQUIRED.filter(name => needs?.[name]?.result !== 'success');
  if (failed.length) throw new Error(`Full validation did not succeed: ${failed.join(', ')}`);
  return true;
}

module.exports = { requireFullValidation };
