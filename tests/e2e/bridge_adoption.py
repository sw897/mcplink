"""Exercise adoption of native Bridges and deletion that preserves direct entries."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess

from run import BINARY, Harness, McpClient


def verify_keep_native(harness, cli, passed, shared, shared_path):
    files = [harness.home / '.hermes/config.yaml', harness.home / '.codex/config.toml']
    direct = cli('connections', 'import', '--source', 'hermes:user', '--name', 'disabled_tool')['id']
    _, applied = harness.changes(direct, ['codex:user'])
    operation = applied['results'][0]['operationId']
    original = [file.read_bytes() for file in files]

    cli('connections', 'delete', direct, fail=True)
    bridge_config = shared_path.read_bytes()
    failure = cli('connections', 'delete', shared, '--keep-native', fail=True)
    assert '共享' in failure['error']['message'] and shared_path.read_bytes() == bridge_config
    assert cli('connections', 'show', shared)['connection']['id'] == shared
    passed('ordinary deletion refuses direct bindings and keep-native refuses mixed direct/shared bindings')

    for replacement in (
        original[0].replace(b'enabled: false', b'enabled: true', 1),
        original[0].replace(b'disabled_tool:', b'renamed_tool:', 1),
        b'mcp_servers: [\n',
        None,
    ):
        if replacement is None:
            files[0].unlink()
        else:
            assert replacement != original[0]
            files[0].write_bytes(replacement)
        try:
            cli('connections', 'delete', direct, '--keep-native', fail=True)
            assert files[1].read_bytes() == original[1]
            assert (files[0].read_bytes() if files[0].exists() else None) == replacement
            assert cli('connections', 'show', direct)['connection']['id'] == direct
            assert sum(b.get('connectionId') == direct for b in cli('status')['bindings']) == 2
        finally:
            files[0].write_bytes(original[0])
    passed('changed or renamed entries, malformed files and missing files retain the connection and all bindings')

    with sqlite3.connect(harness.data / 'mcplink.sqlite3') as db:
        db.execute("CREATE TRIGGER keep_native_fault BEFORE DELETE ON connections BEGIN SELECT RAISE(ABORT,'fixture'); END")
    try:
        cli('connections', 'delete', direct, '--keep-native', fail=True)
        assert cli('connections', 'show', direct)['connection']['id'] == direct
        assert sum(b.get('connectionId') == direct for b in cli('status')['bindings']) == 2
        assert [file.read_bytes() for file in files] == original
    finally:
        with sqlite3.connect(harness.data / 'mcplink.sqlite3') as db:
            db.execute("DROP TRIGGER keep_native_fault")
    passed('a database write failure rolls back all bindings and leaves credentials available')

    assert cli('connections', 'delete', direct, '--keep-native') == {
        'deleted': True, 'nativePreserved': True, 'releasedBindings': 2}
    assert [file.read_bytes() for file in files] == original
    status = cli('status')
    rows = [b for b in status['bindings'] if b['name'] == 'disabled_tool']
    assert len(rows) == 2 and all(b['connectionId'] is None and not b['enabled'] for b in rows)
    assert not any(c['id'] == direct for c in status['connections'])
    cli('changes', 'restore', operation, fail=True)
    assert [file.read_bytes() for file in files] == original
    passed('two direct entries keep their exact bytes and disabled state, and old history cannot remove them')

    catalog = harness.home / 'readonly-catalog.json'
    contents = b'{"mcpServers": {"catalog": {"command": "synthetic-catalog"}}}\n'
    catalog.write_bytes(contents)
    source = harness.rpc('sources.add', {'agent': 'claude', 'path': str(catalog)})
    assert source['readOnly']
    catalog_id = cli('connections', 'import', '--source', source['id'], '--name', 'catalog')['id']
    assert cli('connections', 'delete', catalog_id, '--keep-native')['releasedBindings'] == 1
    assert catalog.read_bytes() == contents
    assert any(c.get('detected') and c.get('sourceId') == source['id'] and c['name'] == 'catalog'
               for c in cli('status')['connections'])
    passed('a read-only source returns to discovery with its original file intact')

    unbound = harness.rpc('connections.save', {'name': 'explicit_flag', 'definition': {'url': harness.url}})['id']
    assert '布尔' in harness.rpc('connections.delete', {'id': unbound, 'keepNative': 'true'}, error=True)
    assert cli('connections', 'show', unbound)['connection']['id'] == unbound
    assert cli('connections', 'delete', unbound) == {'deleted': True}
    passed('malformed keepNative never falls back to ordinary deletion, which still works without bindings')

    direct = cli('connections', 'import', '--source', 'hermes:user', '--name', 'disabled_tool')['id']
    with sqlite3.connect(harness.data / 'mcplink.sqlite3') as db:
        document, = db.execute('SELECT document FROM connections WHERE id=?', (direct,)).fetchone()
    ref = json.loads(document)['definitionRef']
    vault = harness.data / 'fixture-vault'
    definition = vault / hashlib.sha256(ref.encode()).hexdigest()
    grant = vault / hashlib.sha256(f'oauth/{direct}'.encode()).hexdigest()
    grant.mkdir()  # A directory at this fixture key makes credential removal fail.
    try:
        failed = cli('connections', 'delete', direct, '--keep-native', fail=True)
        assert '凭据' in failed['error']['message']
        cli('gateway', 'stop')
        assert not any(c['id'] == direct for c in cli('status')['connections'])
    finally:
        grant.rmdir()
    cli('gateway', 'stop')
    cli('status')
    assert not definition.exists() and not grant.exists(), 'startup did not finish credential cleanup'
    assert cli('connections', 'delete', direct, '--keep-native')['releasedBindings'] == 1
    assert [file.read_bytes() for file in files] == original
    passed('startup completes interrupted vault cleanup and a retry returns the original deletion result')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record', type=Path, required=True)
    record = parser.parse_args().record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = Harness(record)
    scenarios = []

    def cli(*args, fail=False):
        result = subprocess.run([str(BINARY), *args], env=harness.environment,
                                capture_output=True, text=True, timeout=40)
        value = json.loads(result.stdout)
        assert (result.returncode != 0) == fail, value
        return value if fail else value['ok']

    def passed(name):
        scenarios.append({'scenario': name, 'result': 'PASS'})
        print('PASS', name, flush=True)

    conclusion, error = 'PASS', None
    try:
        connection = cli('connections', 'import', '--source', 'codex:user', '--name', 'local_notes')
        identifier = connection['id']
        cli('connections', 'probe', identifier)
        baseline = cli('status')
        copy = record / BINARY.name
        shutil.copy2(BINARY, copy)
        base = {'type': 'stdio', 'command': str(copy),
                'args': ['bridge', '--connection', identifier],
                'env': {'MCPLINK_DATA_DIR': str(harness.data),
                        'MCPLINK_HOME': str(harness.home), 'MCPLINK_VAULT': 'fixture'}}
        native = {**base, 'args': [*base['args'], '--data-dir', str(harness.data)],
                  'env': {k: v for k, v in base['env'].items() if k != 'MCPLINK_DATA_DIR'}}
        path = harness.home / '.claude/.claude.json'
        path.parent.mkdir(exist_ok=True)
        document = {'unrelated': {'keep': 42}, 'mcpServers': {'external_bridge': native}}
        path.write_text(json.dumps(document))
        original = path.read_bytes()
        imported = cli('connections', 'import', '--source', 'claude:user', '--name', 'external_bridge')
        assert imported['id'] == identifier, 'Bridge import duplicated the existing connection'
        status = cli('status')
        binding = next(b for b in status['bindings'] if b['name'] == 'external_bridge')
        assert binding['mode'] == 'shared' and binding['connectionId'] == identifier
        assert not binding['changed'] and not binding['needsUpdate']
        assert len(status['connections']) == len(baseline['connections'])
        assert path.read_bytes() == original
        passed('import identifies an identical CLI copy and attaches the existing shared ID without native writes')
        again = cli('connections', 'import', '--source', 'claude:user', '--name', 'external_bridge')
        assert again['id'] == identifier and again['revision'] == connection['revision']
        passed('repeated import preserves the connection revision and remains idempotent')
        client = McpClient(identifier, harness.environment, 'adopted-native-bridge', native)
        harness.clients.append(client)
        assert len(client.call('tools/list')['result']['tools']) == 2
        passed('the unchanged native Bridge still discovers tools from the managed upstream')
        for name, args in {
            'global_before': ['--data-dir', str(harness.data), *base['args']],
            'global_inside': ['bridge', '--data-dir', str(harness.data), '--connection', identifier],
            'env_data': base['args'],
        }.items():
            document['mcpServers'][name] = {**base, 'args': args}
            path.write_text(json.dumps(document))
            assert cli('connections', 'import', '--source', 'claude:user', '--name', name)['id'] == identifier
        passed('global data-dir options before or after bridge and explicit data environment refer to the same connection')
        fake = record / 'different/mcplink'
        fake.parent.mkdir()
        fake.write_text('#!/bin/sh\nexit 1\n')
        fake.chmod(0o755)
        cases = {
            'other_data': {'env': {**base['env'], 'MCPLINK_DATA_DIR': str(record / 'other-data')}},
            'other_namespace': {'env': {**base['env'], 'MCPLINK_DATA_DIR': str(harness.data) + '/.'}},
            'unknown_id': {'args': ['bridge', '--connection', 'not-a-managed-connection']},
            'different_cli': {'command': str(fake)},
            'extra_args': {'args': [*base['args'], '--port', '1']},
            'extra_env': {'env': {**base['env'], 'UNRELATED_AUTH': 'synthetic-private-value'}},
        }
        for name, change in cases.items():
            document['mcpServers'][name] = {**base, **change}
            path.write_text(json.dumps(document))
            before = path.read_bytes()
            failure = cli('connections', 'import', '--source', 'claude:user', '--name', name, fail=True)
            assert 'synthetic-private-value' not in json.dumps(failure)
            assert path.read_bytes() == before
            assert len([c for c in cli('connections', 'list') if not c.get('detected')]) == 1
        passed('foreign scope, missing ID, changed executable, extra arguments and environment fail without wrappers or writes')
        verify_keep_native(harness, cli, passed, identifier, path)
    except Exception as exception:
        conclusion, error = 'FAIL', str(exception)
        print('FAIL', error, flush=True)
    finally:
        harness.close(conclusion != 'PASS')
    report = {'conclusion': conclusion, 'error': error, 'scenarios': scenarios,
              'executed_at': datetime.now(timezone.utc).isoformat(),
              'binary_sha256': hashlib.sha256(BINARY.read_bytes()).hexdigest(),
              'script_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'scope': 'isolated native config, actual CLI and SDK, fixture credential store'}
    (record / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    raise SystemExit(conclusion != 'PASS')


if __name__ == '__main__':
    main()
