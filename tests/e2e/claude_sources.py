"""Exercise Claude read-only inventory/import through the actual management CLI."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import traceback

from run import BINARY, FIXTURE, ROOT, Harness


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--record', type=Path, required=True)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = Harness(record)
    results = []
    home = harness.home / '.claude'
    home.mkdir()
    user = home / '.claude.json'
    plugin = home / 'plugins/cache/enabled/1.0.0'
    plugin.mkdir(parents=True)
    shutil.copy2(FIXTURE, plugin / 'server.py')
    plugin_config = plugin / '.mcp.json'
    plugin_config.write_text(json.dumps({'plugin_notes': {'type': 'stdio', 'command': sys.executable,
        'args': ['${CLAUDE_PLUGIN_ROOT}/server.py', '--stdio'], 'env': {'FIXTURE_SECRET': 'synthetic-native-value'}}}))
    disabled = home / 'plugins/cache/disabled/1.0.0'
    disabled.mkdir(parents=True)
    (disabled / '.mcp.json').write_text('intentionally invalid disabled plugin JSON')
    user.write_text(json.dumps({'unrelated': {'keep': 42}, 'mcpServers': {'user_notes': {
        'command': sys.executable, 'args': [str(FIXTURE), '--stdio']}, 'unsupported': {'type': 'sse', 'url': harness.url}}}))
    settings = home / 'settings.json'
    settings.write_text(json.dumps({'enabledPlugins': {'enabled@fixture': True, 'disabled@fixture': False, 'project@fixture': True}}))
    registry = home / 'plugins/installed_plugins.json'
    registry.write_text(json.dumps({'version': 2, 'plugins': {
        'enabled@fixture': [{'scope': 'user', 'installPath': str(plugin)}],
        'disabled@fixture': [{'scope': 'user', 'installPath': str(disabled)}],
        'project@fixture': [{'scope': 'project', 'installPath': str(disabled)}]}}))
    originals = {p: p.read_bytes() for p in [user, settings, registry, plugin_config, disabled / '.mcp.json']}
    def passed(name):
        results.append({'scenario': name, 'result': 'PASS'})
        print('PASS', name, flush=True)
    conclusion, error = 'PASS', None
    try:
        status = harness.rpc('snapshot')
        sources = [s for s in status['sources'] if s['agent'] == 'claude']
        assert len(sources) == 2 and not next(s for s in sources if s['id']=='claude:user')['readOnly']
        source = next(s for s in sources if s['path'] == str(plugin_config))
        assert source['readOnly'] and all('disabled' not in s['path'] for s in sources)
        passed('discover Claude camelCase user JSON and enabled user plugin; skip disabled/project plugins')
        managed = harness.rpc('connections.import', {'sourceId': source['id'], 'name': 'plugin_notes'})['id']
        detail = harness.rpc('connections.detail', {'id': managed})
        assert detail['definition']['args'][0] == '••••••••[1]'
        assert detail['definition']['env']['FIXTURE_SECRET'] == '••••••••'
        assert 'synthetic-native-value' not in json.dumps(detail)
        passed('import plugin definition, resolve plugin root and redact configured secret')
        user_id = harness.rpc('connections.import', {'sourceId': 'claude:user', 'name': 'user_notes'})['id']
        assert harness.rpc('connections.probe', {'id': user_id})['connection']['status'] == 'ready'
        assert harness.rpc('connections.probe', {'id': managed})['connection']['status'] == 'ready'
        passed('import both native source shapes and discover actual MCP tools')
        for target in [source['id']]:
            message = harness.rpc('changes.preview', {'connectionId': managed, 'targetSourceIds': [target]}, error=True)
            assert '只读' in message
        added = harness.rpc('sources.add', {'agent': 'hermes', 'path': str(plugin_config)})
        assert added['id'] == source['id'] and added['readOnly']
        passed('reject writes to plugin sources and preserve readonly identity on duplicate path registration')
        unsupported = harness.rpc('connections.import', {'sourceId': 'claude:user', 'name': 'unsupported'}, error=True)
        assert '传输' in unsupported
        passed('unsupported Claude transport is explicit rather than treated as ordinary HTTP')
        plan = harness.rpc('changes.preview', {'connectionId': managed, 'targetSourceIds': ['hermes:user'], 'mode': 'direct'})
        applied = harness.rpc('changes.apply', {'planId': plan['planId']})
        assert applied['results'][0]['status'] == 'applied'
        native = json.loads(subprocess.check_output([
            'node', str(ROOT / 'tests/fixtures/read-native-config.mjs'), 'hermes',
            str(harness.home / '.hermes/config.yaml'), 'plugin_notes',
        ], timeout=20))
        script, mode = native['args']
        assert Path(script).is_absolute() and Path(script).samefile(plugin / 'server.py')
        assert mode == '--stdio'
        harness.rpc('changes.restore', {'operationId': applied['results'][0]['operationId']})
        passed('reuse imported literal/plugin-root definition on writable Hermes source and restore target entry')
        plugin_config.write_text('invalid enabled JSON with synthetic-native-value')
        status = harness.rpc('snapshot')
        assert next(s for s in status['sources'] if s['id'] == source['id'])['error']
        assert all(not s['error'] for s in status['sources'] if s['agent'] in ['codex', 'hermes'])
        assert 'synthetic-native-value' not in json.dumps(status)
        plugin_config.write_bytes(originals[plugin_config])
        registry.write_text('invalid registry')
        status = harness.rpc('snapshot')
        assert next(s for s in status['sources'] if s['id'] == 'claude:plugins:error')['error']
        assert any(c['name'] == 'local_notes' for c in status['connections'])
        registry.write_bytes(originals[registry])
        passed('isolate and sanitize plugin/config discovery errors without hiding other Agent sources')
        assert all(p.read_bytes() == content for p, content in originals.items())
        passed('all original Claude preference and plugin files remain byte-identical after management operations')
    except Exception as exception:
        conclusion, error = 'FAIL', str(exception)
        print('FAIL', error, flush=True)
        traceback.print_exc()
    finally:
        harness.close(False)
    report = {'conclusion': conclusion, 'error': error, 'scenarios': results,
              'executed_at': datetime.now(timezone.utc).isoformat(),
              'binary_sha256': hashlib.sha256(BINARY.read_bytes()).hexdigest(),
              'script_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'scope': 'isolated Claude JSON/plugin settings, real CLI/SDK and fixture Vault; no real native config writes'}
    (record / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print(record, flush=True)
    raise SystemExit(0 if conclusion == 'PASS' else 1)


if __name__ == '__main__':
    main()
