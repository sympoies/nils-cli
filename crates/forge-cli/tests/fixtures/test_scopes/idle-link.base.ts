const assert = require('node:assert/strict');
const { pollHistory } = require('./packages/edge/src/visibility.cjs');

function startFixture() {
  const sessions = [{
    name: 'sample',
    active: true,
    visible: true,
  }];
  let outboxReads = 0;

  const read = (path) => {
    if (path === '/sessions') return { sessions };
    if (path === '/outbox') {
      outboxReads++;
      return { items: [] };
    }
    return { missing: true };
  };

  return {
    read,
    sessions,
    get outboxReads() { return outboxReads; },
  };
}

function test(_name, run) { run(); }

test('visible views poll and hidden views pause', () => {
  const fixture = startFixture();
  let visible = true;
  const refresh = () => {
    if (visible) fixture.read('/outbox');
    pollHistory(visible, fixture.read);
  };

  refresh();
  assert.ok(fixture.outboxReads > 0, 'visible view polls the outbox');
  visible = false;
  const visibleOutboxReads = fixture.outboxReads;
  refresh();
  assert.equal(fixture.outboxReads, visibleOutboxReads, 'hidden view stops polling the outbox');
  visible = true;
  refresh();
  assert.ok(fixture.outboxReads > visibleOutboxReads, 'visible view resumes polling the outbox');
});
