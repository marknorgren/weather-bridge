#!/usr/bin/env python3
"""Build a deliberately allowlisted, portable GitHub Pages artifact."""
import argparse
import html
import json
import re
from pathlib import Path
import shutil
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_API = 'https://bridge.wx.mrkd.co'
# Shown when the build runs without the LikeC4 site, as in a Python-only local preview.
ARCHITECTURE_PLACEHOLDER = '''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Architecture · Weather Bridge</title><link rel="stylesheet" href="../styles.css"></head><body>
<main class="wrap prose" id="main"><h1>Architecture</h1><p>This build does not include the interactive canonical LikeC4 model. The Pages workflow adds it. To include it locally, run <code>likec4 build docs/architecture --output-single-file --use-hash-history -o target/likec4</code>, then <code>python3 scripts/build-docs.py --architecture target/likec4</code>.</p><p>The model source and its maintenance guide are in <code>docs/architecture/</code>.</p><p><a href="../index.html">Back to the docs</a></p></main></body></html>
'''

MARKDOWN_ROOTS = ('docs', 'adrs', 'data', 'infra', 'web/vendor')
ROOT_GUIDES = ('README.md', 'AGENTS.md')
EXCLUDED_MARKDOWN = ('docs/exec-plans/',)


def maintained_markdown(root=ROOT):
    """Discover current public guides without pulling historical execution plans into the contract."""
    documents = [root / name for name in ROOT_GUIDES if (root / name).is_file()]
    for directory in MARKDOWN_ROOTS:
        base = root / directory
        if not base.is_dir():
            continue
        for document in base.rglob('*.md'):
            relative = document.relative_to(root).as_posix()
            if (document.is_file() and not document.is_symlink()
                    and not relative.startswith(EXCLUDED_MARKDOWN)):
                documents.append(document)
    return sorted(set(documents))


def local_link_failures(root=ROOT):
    """Return missing or escaping local Markdown destinations in maintained guides."""
    failures = []
    for document in maintained_markdown(root):
        for destination in re.findall(r'!?\[[^]]+\]\(([^)]+)\)', document.read_text()):
            destination = destination.strip()
            if destination.startswith('<') and '>' in destination:
                link = destination[1:destination.index('>')]
            else:
                link = destination.split(maxsplit=1)[0]
            parsed = urlsplit(link)
            if parsed.scheme or parsed.netloc or not parsed.path:
                continue
            target = (document.parent / unquote(parsed.path)).resolve()
            if root not in target.parents and target != root:
                failures.append(f'{document.relative_to(root)}: link escapes repository: {link}')
            elif not target.exists():
                failures.append(f'{document.relative_to(root)}: missing link target: {link}')
    return failures


def check_local_links(parser, root=ROOT):
    failures = local_link_failures(root)
    if failures:
        parser.error('invalid local Markdown links:\n' + '\n'.join(failures))
    return len(maintained_markdown(root))


p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--output', type=Path, default=ROOT / 'target/docs-site')
p.add_argument('--api-url', default=DEFAULT_API)
p.add_argument('--architecture', type=Path, help='LikeC4 single-file build (likec4 build --output-single-file); its index.html becomes architecture/index.html')
p.add_argument('--check-links', action='store_true', help='check maintained Markdown links and exit without building the site')
a = p.parse_args()
checked_links = check_local_links(p)
if a.check_links:
    print(f'Checked local links in {checked_links} Markdown files')
    raise SystemExit(0)
api = a.api_url.rstrip('/')
parts = urlsplit(api)
if not re.fullmatch(r'https://[A-Za-z0-9.-]+(?::[0-9]{1,5})?', api) or not parts.hostname or parts.path or parts.query or parts.fragment or parts.username:
    p.error('--api-url must be an HTTPS origin without credentials or a path')
if a.architecture and not (a.architecture / 'index.html').is_file():
    p.error('--architecture must be a LikeC4 build directory containing index.html')
out = a.output.resolve()
source = ROOT / 'docs/site'
if out == ROOT or out == source or out in source.parents:
    p.error('output must not overwrite source directories')
allowed = {'index.html', 'rest.html', 'mcp.html', 'styles.css', 'site.js', 'scalar-1.72.1.js', 'SCALAR-LICENSE.txt', 'openapi.json', '.nojekyll'}
def expected(path):
    if path.name == 'architecture' and path.is_dir() and not path.is_symlink():
        return all(child.name == 'index.html' and child.is_file() and not child.is_symlink() for child in path.iterdir())
    return path.name in allowed and path.is_file() and not path.is_symlink()
if out.exists() and not all(expected(path) for path in out.iterdir()):
    p.error('output contains unexpected files; choose a clean directory')
out.mkdir(parents=True, exist_ok=True)
for name in ('index.html', 'rest.html', 'mcp.html', 'styles.css', 'site.js'):
    content = (source / name).read_text()
    (out / name).write_text(content.replace('@@API@@', html.escape(api, quote=True)))
shutil.copyfile(ROOT / 'web/vendor/scalar-1.72.1.js', out / 'scalar-1.72.1.js')
shutil.copyfile(ROOT / 'web/vendor/SCALAR-LICENSE.txt', out / 'SCALAR-LICENSE.txt')
spec = json.loads((ROOT / 'openapi.json').read_text())
spec['servers'] = [{'url': api, 'description': 'Public Weather Bridge demo (AWS)'}]
(out / 'openapi.json').write_text(json.dumps(spec, indent=2) + '\n')
(out / 'architecture').mkdir(exist_ok=True)
if a.architecture:
    shutil.copyfile(a.architecture / 'index.html', out / 'architecture/index.html')
else:
    (out / 'architecture/index.html').write_text(ARCHITECTURE_PLACEHOLDER)
(out / '.nojekyll').touch()
print(f'Built documentation in {out}')
