#!/usr/bin/env python3
"""Merge a validated version-only PR, then tag its merge commit and draft notes."""
import argparse
import base64
import json
import os
import re
import subprocess
import time

from prepare_release import update_manifest
from release import ROOT, REPO, git, github_release, version_tuple
from release_notes_pr import api, pages, marker, BOT_ID, refresh_gate, open_pr

FILES = ('Cargo.toml', 'Cargo.lock')


def next_tag(previous, level):
    parts = list(version_tuple(previous))
    index = {'major': 0, 'minor': 1, 'patch': 2}[level]
    parts[index] += 1
    parts[index + 1:] = [0] * (2 - index)
    return 'v' + '.'.join(map(str, parts))


def version_files(base, tag):
    version_tuple(tag)
    manifest = git(ROOT, 'show', f'{base}:Cargo.toml') + '\n'
    lock = git(ROOT, 'show', f'{base}:Cargo.lock') + '\n'
    # Change only the root package entry, never dependency versions/checksums.
    pattern = r'(?m)(^name = "codex-reset"\nversion = ")[^"]+("$)'
    lock, count = re.subn(pattern, lambda m: m[1] + tag[1:] + m[2], lock)
    if count != 1:
        raise ValueError('Expected exactly one codex-reset lockfile entry')
    return {'Cargo.toml': update_manifest(manifest, tag[1:]), 'Cargo.lock': lock}


def validate_version_pr(pr, data):
    if (pr['user']['id'] != BOT_ID or pr['base']['ref'] != 'main'
            or pr['head']['ref'] != 'release-version/' + data['tag']
            or pr['head']['repo']['full_name'] != REPO
            or pr['base']['repo']['full_name'] != REPO
            or marker('codex-version', data) not in (pr.get('body') or '')):
        raise ValueError('Version PR identity changed; refusing automatic merge')
    changed = pages(f'pulls/{pr["number"]}/files')
    if sorted(f['filename'] for f in changed) != sorted(FILES) or any(f['status'] != 'modified' for f in changed):
        raise ValueError('Version PR must modify only both version files')
    if not re.fullmatch(r'[a-f0-9]{40}', data['base']):
        raise ValueError('Invalid version baseline commit')
    commit = api('git/commits/' + pr['head']['sha'])
    if [parent['sha'] for parent in commit['parents']] != [data['base']]:
        raise ValueError('Version PR head must be the single generated commit')
    subprocess.run(['git', 'merge-base', '--is-ancestor', data['base'], 'origin/main'],
                   cwd=ROOT, check=True, capture_output=True)
    expected = version_files(data['base'], data['tag'])
    if not pr.get('merged') and expected != version_files('origin/main', data['tag']):
        raise ValueError('Main version files changed; recreate the version PR from current main')
    for path, text in expected.items():
        blob = api(f'contents/{path}?ref={pr["head"]["sha"]}')
        if base64.b64decode(blob['content']).decode() != text:
            raise ValueError('Version PR contains unexpected edits')
    return expected


def start(level):
    if os.environ.get('GITHUB_REF') != 'refs/heads/main':
        raise ValueError('Automatic releases must run from main')
    latest = github_release('latest')
    previous = latest['tag_name'] if latest else 'v0.0.0'
    tag = next_tag(previous, level)
    run_id = os.environ['GITHUB_RUN_ID']
    for old in pages('pulls?state=all&base=main'):
        match = re.search(r'<!-- codex-version: (.+?) -->', old.get('body') or '')
        if not match or old['user']['id'] != BOT_ID:
            continue
        saved = json.loads(match[1])
        if saved.get('run') != run_id:
            continue
        tag, previous = saved['tag'], saved['previous']
        if latest and version_tuple(latest['tag_name']) >= version_tuple(tag):
            print(f'{tag} is already released or superseded; this run will not create another version')
            return
        break
    for pr in pages('pulls?state=open&base=main'):
        if pr['head']['ref'].startswith('release-notes/'):
            raise ValueError('Finish the existing release-note PR before starting another release')
        if pr['head']['ref'].startswith('release-version/') and pr['head']['ref'] != 'release-version/' + tag:
            raise ValueError('Another version PR is pending')
    for ref in api('git/matching-refs/tags/v'):
        name = ref['ref'].removeprefix('refs/tags/')
        if re.fullmatch(r'v\d+\.\d+\.\d+', name) and version_tuple(name) > version_tuple(previous) and name != tag:
            raise ValueError('Another unpublished version tag exists')
    branch = 'release-version/' + tag
    existing = api(f'pulls?state=all&head={REPO.split("/")[0]}:{branch}&base=main')
    if existing:
        pr = api(f'pulls/{existing[0]["number"]}')
        match = re.search(r'<!-- codex-version: (.+?) -->', pr.get('body') or '')
        if not match:
            raise ValueError('Existing version PR is not managed by this workflow')
        data = json.loads(match[1])
        if data.get('run') != run_id:
            raise ValueError('Version PR belongs to another run; retry that original Actions run')
        if data.get('tag') != tag or data.get('previous') != previous:
            raise ValueError('Version baseline changed')
    else:
        if api('git/ref/tags/' + tag, missing=True):
            raise ValueError('Unpublished tag already exists; resume its notes workflow instead')
        base = api('git/ref/heads/main')['object']['sha']
        git(ROOT, 'fetch', 'origin', 'main')
        data = dict(tag=tag, previous=previous, base=base, run=run_id)
        expected = version_files(base, tag)
        tree = api('git/trees', dict(base_tree=api('git/commits/' + base)['tree']['sha'],
                    tree=[dict(path=p, mode='100644', type='blob', content=t) for p, t in expected.items()]))
        commit = api('git/commits', dict(message=f'Bump version to {tag[1:]}\n\nUpdate package and lockfile versions for release.\nValidation: exact version-only transformation; dependencies unchanged.',
                                       tree=tree['sha'], parents=[base]))
        api('git/refs', dict(ref='refs/heads/' + branch, sha=commit['sha']))
        pr = api('pulls', dict(title='Prepare release ' + tag, head=branch, base='main',
                 body='Automatically merge after exact version-file validation. Release notes still require human approval.\n\n' + marker('codex-version', data)))
        pr = api(f'pulls/{pr["number"]}')
    git(ROOT, 'fetch', 'origin', 'main')
    expected = validate_version_pr(pr, data)
    if not pr.get('merged'):
        if pr['state'] != 'open':
            raise ValueError('Version PR was closed without merging')
        refresh_gate(pr['number'])
        api('statuses/' + pr['head']['sha'], dict(state='success', context='Release version validation',
            description='Exact package and lockfile version updates verified'))
        # Respect GitHub branch rules; never bypass checks or force the base branch.
        for _ in range(12):
            current = api(f'pulls/{pr["number"]}')
            if current['head']['sha'] != pr['head']['sha']:
                raise ValueError('Version PR changed during validation')
            if current.get('mergeable_state') == 'clean':
                break
            time.sleep(5)
        else:
            raise ValueError('Version PR is blocked by branch checks; inspect it and retry')
        if (github_release('latest') or {}).get('tag_name', 'v0.0.0') != previous:
            raise ValueError('Latest release changed during preparation')
        result = api(f'pulls/{pr["number"]}/merge', dict(sha=pr['head']['sha'], merge_method='merge'), 'PUT')
        if not result.get('merged'):
            raise ValueError('GitHub did not merge the version PR')
        merged = result['sha']
    else:
        merged = pr['merge_commit_sha']
    git(ROOT, 'fetch', 'origin', 'main')
    for path, text in expected.items():
        if git(ROOT, 'show', f'{merged}:{path}') + '\n' != text:
            raise ValueError('Merge result differs from validated version files')
    ref = api('git/ref/tags/' + tag, missing=True)
    if ref is None:
        api('git/refs', dict(ref='refs/tags/' + tag, sha=merged))
    elif ref['object']['sha'] != merged:
        raise ValueError('Existing tag does not match the version merge')
    git(ROOT, 'fetch', 'origin', f'refs/tags/{tag}:refs/tags/{tag}')
    git(ROOT, 'checkout', '--detach', tag)
    # GITHUB_TOKEN merges/tags do not trigger another workflow: continue here.
    open_pr(tag)
    print(f'Version PR #{pr["number"]} merged; {tag} created at {merged}')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--level', choices=['major', 'minor', 'patch'], required=True)
    start(parser.parse_args().level)
