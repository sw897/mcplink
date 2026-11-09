"""Import external native Bridges without duplicating their managed upstream."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

from run import BINARY, Harness, McpClient


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
