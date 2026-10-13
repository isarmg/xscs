import assert from 'node:assert/strict';
import { test } from 'node:test';
import { configValueLabel, operationLabel } from '../src/display-labels.ts';

test('unknown task labels have a fallback and custom config values remain unchanged', () => {
  assert.equal(operationLabel('sunshine.read_config'), 'Read configuration');
  assert.equal(operationLabel('device_revoked'), 'Instance revoked');
  assert.equal(operationLabel('authorization_rotated'), 'Instance password changed');
  assert.equal(operationLabel('execution_deadline_expired'), 'Task dispatch deadline expired without execution');
  assert.equal(configValueLabel('info'), 'Information');
  for (const value of ['__proto__', 'constructor', 'toString']) {
    assert.equal(operationLabel('sunshine.' + value), 'Unrecognized state');
    assert.equal(configValueLabel(value), value);
  }
});
