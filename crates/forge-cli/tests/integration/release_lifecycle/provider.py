#!/usr/bin/env python3
"""Stateful GitHub REST fixture; never contacts a provider."""
import json
import os
from pathlib import Path
import sys
from urllib.parse import unquote

args = sys.argv[1:]
root = Path(os.environ['RELEASE_FIXTURE'])
with (root / 'calls.jsonl').open('a') as log:
    log.write(json.dumps({'argv': args, 'actor': os.environ.get('GH_TOKEN')}) + '\n')
if args[:2] == ['api', 'user']:
    print(json.dumps({'login': 'fixture-reader'}))
    sys.exit(0)
if args[:2] == ['api', 'repos/example/widget']:
    print(json.dumps({'full_name': 'example/widget', 'permissions': {'push': True}}))
    sys.exit(0)
assert args[0] == 'api', args
if args[1] == 'graphql':
    state = json.loads((root / 'state.json').read_text())
    tag = next(arg.removeprefix('tag=') for arg in args if arg.startswith('tag='))
    release = {'databaseId': state['release']['id']} if state['release_exists'] and tag == state['release']['tag_name'] else None
    if '--include' in args:
        print('HTTP/2.0 200 Fixture\r\nContent-Type: application/json\r\n\r\n', end='')
    print(json.dumps({'data': {'repository': {'release': release}}}))
    sys.exit(0)
path = next(arg for arg in args if arg.startswith('repos/'))
method = args[args.index('--method') + 1] if '--method' in args else 'GET'
mode = os.environ.get('RELEASE_MODE', '')
state = json.loads((root / 'state.json').read_text())

def reply(status, body=None):
    if '--include' in args:
        print(f'HTTP/2.0 {status} Fixture\r\nContent-Type: application/json\r\n\r\n', end='')
    if body is not None:
        print(json.dumps(body, ensure_ascii=False))
    sys.exit(0 if status < 400 else 1)

if mode == 'network' or (mode == 'readback-fails' and state['mutated']):
    reply(503, {'message': 'fixture unavailable'})
if path.startswith('repos/example/widget/releases/'):
    suffix = unquote(path.removeprefix('repos/example/widget/releases/'))
    if suffix not in ('17', 'tags/' + state['release']['tag_name']):
        reply(404, {'message': 'Not Found'})
    if suffix.startswith('tags/') and state['release']['draft']:
        reply(404, {'message': 'Not Found'})
    if mode == 'wrong-release':
        state['release']['tag_name'] = 'other-tag'
    if method == 'PATCH':
        payload = json.loads(Path(args[args.index('--input') + 1]).read_text())
        (root / 'payload.json').write_text(json.dumps(payload, ensure_ascii=False))
        assert set(payload) <= {'body', 'name'}, payload
        if mode != 'edit-mismatch':
            state['release'].update(payload)
        if mode == 'edit-state-changed':
            state['release']['draft'] = not state['release']['draft']
        state['mutated'] = True
        (root / 'state.json').write_text(json.dumps(state))
        reply(200, state['release'])
    if method == 'DELETE':
        if mode == 'delete-rejected':
            reply(403, {'message': 'Forbidden'})
        if mode != 'release-remains':
            state['release_exists'] = False
        state['mutated'] = True
        (root / 'state.json').write_text(json.dumps(state))
        reply(204)
    if not state['release_exists']:
        if mode == 'release-recreated' and suffix.startswith('tags/'):
            reply(200, dict(state['release'], id=18))
        reply(404, {'message': 'Not Found'})
    reply(200, state['release'])
if path.startswith('repos/example/widget/git/ref'):
    prefix = 'repos/example/widget/git/refs/tags/' if method == 'DELETE' else 'repos/example/widget/git/ref/tags/'
    assert path.startswith(prefix), path
    assert unquote(path.removeprefix(prefix)) == state['release']['tag_name'], path
    if method == 'DELETE':
        if mode == 'tag-rejected':
            reply(403, {'message': 'Forbidden'})
        if mode != 'tag-remains':
            state['tag_exists'] = False
        (root / 'state.json').write_text(json.dumps(state))
        reply(204)
    if not state['tag_exists']:
        reply(404, {'message': 'Not Found'})
    tag = state['release']['tag_name']
    reply(200, {'ref': 'refs/tags/' + ('other-tag' if mode == 'wrong-ref' else tag),
                'object': {'type': 'tag' if mode == 'annotated' else 'commit',
                           'sha': ('d' if mode == 'tag-moved' and state['mutated'] else ('b' if mode == 'annotated' else 'a')) * 40}})
if path == 'repos/example/widget/git/tags/' + 'b' * 40:
    reply(200, {'sha': 'b' * 40, 'object': {'type': 'commit', 'sha': 'a' * 40}})
if path == 'repos/example/widget/commits/main':
    reply(200, {'sha': 'c' * 40})
raise AssertionError(args)
