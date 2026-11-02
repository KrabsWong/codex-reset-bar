#!/usr/bin/env python3
"""Trusted pre-push guard for the release-note bot; installed outside its checkout."""
import os
import re
import subprocess
import sys

SECRET = re.compile(r'-----BEGIN .*PRIVATE KEY-----|\b(?:gh[pousr]_|github_pat_|sk-|oc_sk_|AKIA)[A-Za-z0-9_-]{16,}|https?://[^\s/@]+:[^\s/@]+@|(?i:authorization\s*[:=]\s*[\"\']?bearer\s+\S+)|(?i:(?:api[_-]?key|password|secret|token)\s*[:=]\s*[\"\'][^\"\']{12,}[\"\'])')


def git(*args):
    return subprocess.check_output(['git', *args], text=True)


def check(lines, tag):
    if not re.fullmatch(r'v\d+\.\d+\.\d+', tag):
        raise ValueError('Invalid note version')
    path = f'release-notes/{tag}.md'
    for line in lines:
        local_ref, local, remote_ref, remote = line.split()
        if remote_ref != f'refs/heads/release-notes/{tag}' or '0' * 40 in (local, remote):
            raise ValueError('Only updates to the existing release-note branch are allowed')
        subprocess.run(['git', 'merge-base', '--is-ancestor', remote, local], check=True, capture_output=True)
        for commit in git('rev-list', f'{remote}..{local}').splitlines():
            paths = git('diff-tree', '--root', '--no-commit-id', '--name-only', '-r', commit).splitlines()
            if paths != [path]:
                raise ValueError('Each outgoing commit must change only its release-note file')
            if SECRET.search(git('show', f'{commit}:{path}')) or SECRET.search(git('show', '-s', '--format=%B', commit)):
                raise ValueError('Possible credential in outgoing history; push blocked')


if __name__ == '__main__':
    try:
        check(sys.stdin, os.environ['RELEASE_NOTE_TAG'])
    except (ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'Release-note push rejected: {type(error).__name__}. Inspect locally; no secret values logged.', file=sys.stderr)
        sys.exit(1)
