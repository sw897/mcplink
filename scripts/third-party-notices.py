"""Collect installed dependency notices for redistribution; uses locked local sources."""
import json
from pathlib import Path
import subprocess
import sys

if sys.argv[1:] == ['--check']:
    from release_manifest import release_version
    print('Verified locked dependency notices for mcplink', release_version())
    raise SystemExit(0)
if sys.argv[1:]:
    raise SystemExit('Usage: python3 scripts/third-party-notices.py [--check]')

root=Path(__file__).resolve().parents[1]
metadata=json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version','1'],cwd=root,stderr=subprocess.DEVNULL))
items=[]
for package in metadata['packages']:
    if package['source']:
        items.append((package['name']+' '+package['version'],package.get('license'),Path(package['manifest_path']).parent))
packages=json.loads((root/'package-lock.json').read_text())['packages']
for relative,package in packages.items():
    if relative and not package.get('dev'):
        items.append((relative.removeprefix('node_modules/')+' '+package['version'],package.get('license'),root/relative))
version=json.loads((root/'package.json').read_text())['version']
blocks=[f'Third-party notices for mcplink {version}\nGenerated from installed locked dependencies.\n']
for name,license_name,directory in sorted(items,key=lambda item:item[0]):
    files=sorted({p for pattern in ['LICENSE*','LICENCE*','COPYING*','NOTICE*','OFL*'] for p in directory.glob(pattern) if p.is_file()})
    blocks.append('\n'+'='*72+'\n'+name+'\nSPDX: '+str(license_name)+'\n')
    for file in files:
        blocks.append('\n--- '+file.name+' ---\n'+file.read_text(errors='replace')+'\n')
(root/'THIRD_PARTY_NOTICES.txt').write_text(''.join(blocks))
print('Collected notices for',len(items),'locked dependencies')
