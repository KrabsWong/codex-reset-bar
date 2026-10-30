#!/usr/bin/env python3
"""Validate a release tag on main and synchronize the disposable CI checkout."""
import argparse
import json
from pathlib import Path
import re
import subprocess

from release import ROOT, validate_tag


def update_manifest(text, version):
    section = re.search(r'(?ms)^\[package\][ \t]*\n(.*?)(?=^\[|\Z)', text)
    if not section:
        raise ValueError('Missing [package] section in Cargo.toml')
    updated, count = re.subn(r'(?m)^(version\s*=\s*)"[^"]+"',
                             lambda match: match[1] + '"' + version + '"', section[1])
    if count != 1:
        raise ValueError('Expected one explicit package version in Cargo.toml')
    return text[:section.start(1)] + updated + text[section.end(1):]


def prepare(root, tag, main_ref='refs/remotes/origin/main'):
    commit = validate_tag(root, tag, main_ref)
    manifest = root / 'Cargo.toml'
    manifest.write_text(update_manifest(manifest.read_text(), tag[1:]))
    # Update workspace versions without upgrading dependencies already in the lockfile.
    subprocess.run(['cargo', 'update', '--workspace'], cwd=root, check=True, capture_output=True)
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--no-deps', '--format-version=1'], cwd=root, text=True))
    package = next(p for p in metadata['packages'] if p['name'] == 'codex-reset')
    if package['version'] != tag[1:]:
        raise ValueError('Cargo package version did not synchronize with tag')
    print(f'Building {tag} from main commit {commit}; version changes stay in this checkout')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag', required=True)
    args = parser.parse_args()
    prepare(ROOT, args.tag)


if __name__ == '__main__':
    main()
