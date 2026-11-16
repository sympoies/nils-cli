"""Synthetic CLI acceptance for private aggregate retirement counts."""
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

PROBE = Path(__file__).with_name('state-retirement-probe.py')
CANARY = 'omitted-fixture-value'


def record(**fields):
    return dict(schema_version='agent-session.session.v1', agent='claude',
                title_revision=0, runtime={'launch_id': CANARY}, **fields)


def terminal():
    return dict(schema_version='agent-session.auto-resume.v1', enabled=False,
                state='terminal_failure', updated_at='2026-01-01T00:00:00Z',
                failure_reason='usage_window_not_exhausted', blocked_turn_id=CANARY,
                blocked_revision=0)


def title_state(topic='Fix #12a', **fields):
    return {'topic': topic, 'topic_source': 'user', 'references': ['#12'],
            'activity': None, **fields}


class StateProbeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / 'sessions').mkdir()
        (self.root / 'coordination').mkdir()

    def write(self, name, value):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value), encoding='utf-8')
        path.chmod(0o600)
        return path

    def probe(self, expected=0, root=None):
        run = subprocess.run([sys.executable, str(PROBE), '--state-root', str(root or self.root),
                              '--now-epoch', '1800000000'], capture_output=True, text=True,
                             timeout=5, check=False)
        self.assertEqual(run.returncode, expected, run.stderr)
        self.assertEqual(run.stderr, '')
        for value in (str(self.root), CANARY):
            self.assertNotIn(value, run.stdout)
        return json.loads(run.stdout)

    def counts(self, store):
        return self.probe()['stores'][store]['counts']

    def test_terminal_predicate_provider_split_absence_and_malformed_fields(self):
        for i, agent in enumerate(('claude', 'codex', CANARY)):
            data = record()
            data['agent'] = agent
            self.write(f'sessions/{i}/session.json', data)
            self.write(f'sessions/{i}/auto-resume.json', terminal())
        self.assertEqual(self.counts('auto_resume')['recovery_terminals'],
                         {'claude': 1, 'codex': 1, 'other': 1})
        for field, value in (('enabled', True), ('state', 'waiting'), ('failure_reason', None),
                             ('blocked_turn_id', None), ('blocked_revision', None)):
            state = terminal()
            state[field] = value
            self.write('sessions/0/auto-resume.json', state)
            self.assertEqual(self.counts('auto_resume')['other_records'], 1)
        for field, value in (('blocked_revision', True), ('blocked_turn_id', []), ('enabled', 0)):
            state = terminal()
            state[field] = value
            self.write('sessions/0/auto-resume.json', state)
            self.assertEqual(self.counts('auto_resume')['invalid_records'], 1)

    def test_switch_intent_state_and_identity_matrix(self):
        for state in ('queued', 'applying', 'failed'):
            for i, fields in enumerate(({}, {'intent_id': None}, {'intent_id': 'intent-1'},
                                        {'intent_id': ''}, {'intent_id': 42})):
                self.write(f'sessions/{state}{i}/session.json', record(codex_account_next={
                    'schema_version': 'agent-session.codex-account-next.v1', 'state': state,
                    'account': CANARY, **fields}))
        self.write('sessions/absent/session.json', record())
        self.write('sessions/invalid/session.json', record(codex_account_next=None))
        counts = self.counts('sessions')['switch_intents']
        self.assertEqual((counts['absent'], counts['invalid']), (1, 1))
        for state in ('queued', 'applying', 'failed'):
            self.assertEqual(counts[state], {'missing': 1, 'null': 1, 'present': 1, 'invalid': 2})

    def test_title_renderer_release_boundaries_and_normalization(self):
        fixtures = [
            (title_state('Parser #317alpha', references=['#317'], activity='Implement fix'), 'Parser #317alpha - Implement fix', 'old_only'),
            (title_state('Parser #317_foo', references=['#317'], activity='Implement fix'), 'Parser #317_foo - Implement fix', 'old_only'),
            (title_state('Parser #317alpha', references=['#317', '#42'], activity='Implement fix'), 'Parser #317alpha #42 - Implement fix', 'old_only'),
            (title_state('Parser #317', references=['#317'], activity='Implement fix'), 'Parser #317 - Implement fix', 'current'),
            (title_state(), 'Fix #12a', 'old_only'),
            (title_state(), 'Fix #12a #12', 'current'),
            (title_state('Fix #123'), 'Fix #123 #12', 'current'),
            (title_state('Fix #12_'), 'Fix #12_', 'old_only'),
            (title_state('Fix #12é'), 'Fix #12é', 'current'),
            (title_state('\ufeffFix\u00a0#12a  '), 'Fix #12a', 'old_only'),
            (title_state(references=['#12', '#12']), 'Fix #12a', 'old_only'),
            (title_state(), ' Fix #12a ', 'mismatch'),
            (title_state(references=['#0']), 'bad', 'invalid'),
            (title_state(topic_source='none'), 'bad', 'invalid'),
            (title_state('x' * 120, activity='y' * 120),
             'x' * 78 + '... #12 - ' + 'y' * 29 + '...', 'current'),
            (title_state('🙂' * 120, references=[]), '🙂' * 120, 'current'),
            (title_state(None, topic_source='none', references=[], activity='Work'), 'Work', 'current'),
        ]
        for state, title, expected in fixtures:
            with self.subTest(expected=expected, title=title):
                self.write('sessions/one/session.json', record(title_state=state, title=title))
                self.assertEqual(self.counts('sessions')['title_pairs'][expected], 1)
        self.write('sessions/one/session.json', record())
        self.assertEqual(self.counts('sessions')['title_pairs']['unstructured'], 1)

    def test_title_fences_are_raw_revision_and_runtime_launch_id(self):
        for field, values in (('title_revision', (None, 0, True, -1)),
                              ('runtime', (None, {'launch_id': None}, {'launch_id': ''}, {'launch_id': CANARY}))):
            for i, value in enumerate(values):
                data = record()
                data[field] = value
                self.write(f'sessions/{field}{i}/session.json', data)
        self.write('sessions/missing/session.json', {'schema_version': 'agent-session.session.v1', 'agent': 'claude'})
        counts = self.counts('sessions')
        self.assertEqual(counts['title_revision'], {'missing': 1, 'null': 1, 'present': 5, 'invalid': 2})
        self.assertEqual(counts['session_incarnation'], {'missing': 2, 'null': 1, 'present': 5, 'invalid': 1})

    def test_empty_missing_root_and_absent_sidecar_are_distinct(self):
        report = self.probe()
        self.assertEqual(report['stores']['sessions']['counts']['records'], 0)
        self.assertEqual(report['stores']['registry'], {'status': 'unobserved', 'counts': None})
        self.assertEqual(report['client_contracts'], {'status': 'unobserved', 'counts': None})
        report = self.probe(2, self.root / 'missing')
        self.assertEqual(report['status'], 'unobserved')
        self.assertTrue(all(v == {'status': 'unobserved', 'counts': None} for v in report['stores'].values()))
        self.assertFalse((self.root / 'missing').exists())
        self.write('sessions/one/session.json', record())
        self.assertEqual(self.counts('auto_resume')['absent_files'], 1)

    def test_strict_json_at_every_store_and_no_partial_counts(self):
        for path, section, value in (
            ('sessions/one/session.json', 'sessions', record()),
            ('sessions/one/auto-resume.json', 'auto_resume', terminal()),
            ('coordination/registry.json', 'registry', {}),
            ('coordination/broker-state.json', 'heartbeat', {}),
            ('coordination/federation-journal.json', 'journal', {}),
        ):
            self.write('sessions/one/session.json', record())
            target = self.write(path, value)
            bad = [b'\xff', json.dumps(value).encode('utf-16'), b'[]', b'{',
                   b'{"private":{"key":1,"key":2}}', b'{"key":1,"key":2}',
                   b'{"private":"\\ud800"}']
            bad.extend(('{"private":' + token + '}').encode() for token in ('NaN', 'Infinity', '-Infinity', '1e999'))
            for content in bad:
                with self.subTest(store=section, content=content):
                    target.write_bytes(content)
                    self.assertEqual(self.probe(2)['stores'][section], {'status': 'invalid', 'counts': None})
                    self.assertEqual(target.read_bytes(), content)
            target.write_text(json.dumps(value))
        self.write('sessions/bad/session.json', {'schema_version': CANARY})
        self.assertIsNone(self.probe(2)['stores']['sessions']['counts'])

    def test_untrusted_nonregular_missing_and_symlink_paths_are_unobserved(self):
        path = self.write('sessions/one/session.json', record())
        path.chmod(0o644)
        self.assertEqual(self.probe()['stores']['sessions']['status'], 'unobserved')
        path.unlink()
        target = self.write('private.json', record())
        path.symlink_to(target)
        self.assertEqual(self.probe()['stores']['sessions']['status'], 'unobserved')
        path.unlink()
        os.mkfifo(path, 0o600)
        self.assertEqual(self.probe()['stores']['sessions']['status'], 'unobserved')
        path.unlink()
        self.assertEqual(self.probe()['stores']['sessions']['status'], 'unobserved')
        path.parent.rmdir()
        path.parent.symlink_to(self.root, target_is_directory=True)
        self.assertEqual(self.probe()['stores']['sessions']['status'], 'unobserved')
        self.assertEqual(self.probe(2, path.parent)['status'], 'unobserved')

    def test_no_state_changes_or_writer_lock_and_no_value_output(self):
        self.write('sessions/one/session.json', record(title=CANARY))
        self.write('sessions/one/auto-resume.json', terminal())
        self.write('coordination/registry.json', {'private': CANARY})
        files = [p for p in self.root.rglob('*') if p.is_file()]
        before = {p: (p.read_bytes(), p.stat()) for p in files}
        lock_path = self.root / 'coordination/registry.lock'
        with lock_path.open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.probe()
        for path, (data, info) in before.items():
            self.assertEqual(path.read_bytes(), data)
            for field in ('st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns', 'st_mode'):
                self.assertEqual(getattr(path.stat(), field), getattr(info, field))
        self.assertEqual(set(p for p in self.root.rglob('*') if p.is_file()), set(files) | {lock_path})

    def test_registry_operation_identity_and_mailbox_forms(self):
        self.write('coordination/registry.json', {
            'schema_version': 'agent-session.coordination-registry.v2',
            'operations': [{'state': 'expired', **field} for field in
                           ({}, {'terminal_at_epoch': None}, {'terminal_at_epoch': 1}, {'terminal_at_epoch': 'bad'})]
                          + [{'state': 'active'}, {'state': 'completing'}, {'state': 'reconcile_pending'}, {}],
            'brokers': {'a': {}, 'b': {'runtime_identity': None, 'binary_version': None},
                        'c': {'runtime_identity': {}, 'binary_version': CANARY},
                        'd': {'runtime_identity': [], 'binary_version': 1}},
            'completion_events': [{}], 'messages': [{},
                {'created_at_epoch_millis': 0, 'created_at_epoch': 1},
                {'created_at_epoch_millis': 1, 'created_at_epoch': None},
                {'created_at_epoch_millis': -1, 'created_at_epoch': True}]})
        counts = self.counts('registry')
        self.assertEqual(counts['schema']['v2'], 1)
        self.assertEqual(counts['operations'], {'total': 8, 'other': 3, 'invalid': 1,
                         'expired_terminal_at': {'missing': 1, 'null': 1, 'present': 1, 'invalid': 1}})
        self.assertEqual(counts['brokers']['runtime_identity'], counts['brokers']['binary_version'])
        self.assertEqual(counts['brokers']['runtime_identity'], {'missing': 1, 'null': 1, 'present': 1, 'invalid': 1})
        self.assertEqual(counts['completion_events'], 1)
        self.assertEqual(counts['mailbox']['millis'], {'missing': 1, 'zero': 1, 'positive': 1, 'invalid': 1})
        self.assertEqual(counts['mailbox']['seconds'], {'missing': 1, 'null': 1, 'present': 1, 'invalid': 1})

    def test_heartbeat_stamp_presence_and_pending(self):
        for field, expected in (({}, 'missing'), ({'completion_registry': None}, 'null'),
                                ({'completion_registry': {}}, 'present'), ({'completion_registry': False}, 'invalid')):
            self.write('coordination/broker-state.json', {'schema_version': 'agent-session.broker-state.v1',
                       'owners': {'private': {}}, 'completion_pending': True, **field})
            self.assertEqual(self.counts('heartbeat')['completion_registry'], expected)
        for pending, expected in ((False, 'false'), (True, 'true'), (None, 'invalid')):
            self.write('coordination/broker-state.json', {'completion_pending': pending})
            self.assertEqual(self.counts('heartbeat')['completion_pending'], expected)

    def test_journal_envelope_schema_category_forwarding_and_totals(self):
        for version in range(1, 7):
            self.write('coordination/federation-journal.json', {
                'schema_version': f'agent-session.federation-journal.v{version}',
                'remote_outbox': [{'state': 'queued', 'envelope': {
                    'schema_version': 'agent-session.remote-message.v1', 'body': CANARY}},
                    {'envelope': {'schema_version': 'agent-session.remote-message.v2', 'category': 'report', 'forwarding': {}}},
                    {'envelope': {'schema_version': 'agent-session.remote-service-message.v2', 'category': None, 'forwarding': None}},
                    {'envelope': {'schema_version': CANARY, 'category': CANARY, 'forwarding': []}}, {'envelope': None}],
                'retained': [{'private': CANARY}]})
            counts = self.counts('journal')
            self.assertEqual(counts['schema'][f'v{version}'], 1)
            self.assertEqual((counts['outbox'], counts['queued'], counts['retained'], counts['invalid_envelopes']), (5, 1, 1, 1))
            self.assertEqual(counts['envelope_schema']['other'], 1)
            self.assertEqual(counts['forwarding'], {'missing': 1, 'null': 1, 'present': 1, 'invalid': 1})
            for key in ('missing', 'null', 'other', 'report'):
                self.assertEqual(counts['category'][key], 1)

    def test_retitle_marker_schema_and_missing_observation(self):
        self.write('sessions/absent/session.json', record())
        self.write('sessions/invalid/session.json', record(session_retitle_v2=None))
        self.write('sessions/marker/session.json', record(session_retitle_v2={
            'schema_version': 'agent-session.session-retitle-state.v2', 'receipts': [
                {}, {'observation': None}, {'observation': {}}, {'observation': []}]}))
        counts = self.counts('sessions')['retitle_markers']
        self.assertEqual((counts['absent'], counts['invalid'], counts['receipts']), (1, 1, 4))
        self.assertEqual(counts['schema']['v2'], 1)
        self.assertEqual(counts['observation'], {'missing': 1, 'null': 1, 'present': 1, 'invalid': 1})

    def test_malformed_store_collections_never_certify_zero(self):
        for name, section, key, bad in (
            ('registry.json', 'registry', 'operations', {}), ('registry.json', 'registry', 'brokers', []),
            ('registry.json', 'registry', 'messages', [None]), ('broker-state.json', 'heartbeat', 'owners', None),
            ('federation-journal.json', 'journal', 'remote_outbox', {}), ('federation-journal.json', 'journal', 'retained', [None])):
            with self.subTest(section=section, key=key):
                path = self.write('coordination/' + name, {key: bad})
                self.assertEqual(self.probe(2)['stores'][section], {'status': 'invalid', 'counts': None})
                path.unlink()


if __name__ == '__main__':
    unittest.main()
