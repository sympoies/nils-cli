const assert = require('node:assert/strict');
const { pollHistory } = require('./packages/edge/src/visibility.cjs');

function startFixture() {
  const sessions = [{
    name: 'sample',
    providerResume: { id: 'sample' },
    active: true,
    visible: true,
  }];
  let outboxReads = 0;
  let historyReads = 0;

  const read = (path) => {
    if (path.startsWith('/history/')) historyReads++;
    if (path === '/sessions') return { sessions };
    if (path === '/history/items') return { items: [{ name: 'sample' }] };
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
    get historyReads() { return historyReads; },
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
  const visibleHistoryReads = fixture.historyReads;
  assert.ok(visibleHistoryReads > 0, 'visible view polls history');
  visible = false;
  const visibleOutboxReads = fixture.outboxReads;
  refresh();
  assert.equal(fixture.outboxReads, visibleOutboxReads, 'hidden view stops polling the outbox');
  assert.equal(fixture.historyReads, visibleHistoryReads, 'hidden view must stop polling history');
  visible = true;
  refresh();
  if (visible) {
    assert.ok(fixture.historyReads > visibleHistoryReads, 'visible view resumes polling history');
  }
  assert.ok(fixture.outboxReads > visibleOutboxReads, 'visible view resumes polling the outbox');
});
