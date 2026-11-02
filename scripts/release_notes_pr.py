#!/usr/bin/env python3
"""Create/revise release-note PRs and authorize a merged, explicitly approved draft."""
import argparse
import base64
import json
import os
from pathlib import Path
import re
import subprocess

from generate_release_summary import comparison, generate
from release import (DEFAULT_RELEASE_SUMMARY, REPO, ROOT, git, github_release,
                     validate_release_summary, validate_tag, version_tuple)

BOT_ID = 41898282  # github-actions[bot]
MARKER = 'codex-release'


def api(endpoint, data=None, method=None, missing=False):
    command = ['gh', 'api', f'repos/{REPO}/{endpoint}']
    if data is not None:
        command += ['--input', '-']
    if method:
        command += ['--method', method]
    result = subprocess.run(command, input=json.dumps(data) if data is not None else None,
                            capture_output=True, text=True)
    if result.returncode:
        if missing and '(HTTP 404)' in result.stderr:
            return None
        raise RuntimeError('GitHub API request failed; no publication authorized')
    return json.loads(result.stdout) if result.stdout.strip() else None


def pages(endpoint):
    result = []
    for page in range(1, 101):
        rows = api(f'{endpoint}?per_page=100&page={page}')
        result.extend(rows)
        if len(rows) < 100:
            return result
    raise ValueError('Too many pages to validate safely')


def marker(kind, value):
    return f'<!-- {kind}: {json.dumps(value, sort_keys=True)} -->'


def metadata(pr):
    match = re.search(r'<!-- codex-release: (.+?) -->', pr.get('body') or '')
    if not match:
        raise ValueError('Not a release-notes PR')
    data = json.loads(match[1])
    version_tuple(data['tag'])
    if data['previous'] is not None:
        version_tuple(data['previous'])
    if not re.fullmatch(r'[a-f0-9]{40}', data['commit']):
        raise ValueError('Invalid release commit')
    if (pr['base']['ref'] != 'main' or pr['head']['ref'] != 'release-notes/' + data['tag']
            or pr['head']['repo']['full_name'] != REPO or pr['base']['repo']['full_name'] != REPO
            or pr['user']['id'] != BOT_ID):
        raise ValueError('Release PR identity does not match')
    return data


def validate_pr(pr):
    data = metadata(pr)
    if git(ROOT, 'rev-parse', '--verify', f'refs/tags/{data["tag"]}^{{commit}}') != data['commit']:
        raise ValueError('Release tag moved after drafting')
    path = f'release-notes/{data["tag"]}.md'
    files = pages(f'pulls/{pr["number"]}/files')
    if len(files) != 1 or files[0]['filename'] != path or files[0]['status'] not in ('added', 'modified'):
        raise ValueError('Release PR must change only its release-note file')
    content = api(f'contents/{path}?ref={pr["head"]["sha"]}')
    summary = validate_release_summary(base64.b64decode(content['content']).decode('utf-8'))
    return data, summary, content['sha']


def draft(data, current=None, feedback=''):
    try:
        context = comparison(ROOT, data['tag'], data['previous'])
        if current:
            context += f'\n\nCurrent draft:\n{current}\n\nMaintainer revision request:\n{feedback}'
        return generate(context, os.environ.get('OPENCODE_MODEL', ''),
                        os.environ.get('OPENCODE_GO_API_KEY', ''))
    except (OSError, ValueError, RuntimeError, KeyError, TypeError, subprocess.SubprocessError):
        print('::warning::AI generation unavailable; retaining existing draft or using fixed fallback.')
        return current if current is not None else DEFAULT_RELEASE_SUMMARY


def write_notes(branch, tag, summary, sha=None):
    # The API commits exactly this reviewed-format text, never a workspace or AI edits.
    summary = validate_release_summary(summary)
    if re.search(r'-----BEGIN .*PRIVATE KEY-----|\b(?:gh[pousr]_|github_pat_|sk-|oc_sk_)[A-Za-z0-9_-]{16,}', summary):
        raise ValueError('Possible credential in release notes; refusing to upload')
    data = dict(branch=branch, message=f'Draft release notes for {tag}\n\n'
                'Summarize user-visible changes for release review.\n'
                'Validation: release-note Markdown format checked; publication awaits approval and merge.',
                content=base64.b64encode(summary.encode()).decode())
    if sha:
        data['sha'] = sha
    return api(f'contents/release-notes/{tag}.md', data, 'PUT')


def start_release(tag):
    """Create an immutable tag, then draft in this run (bot tags do not trigger CI)."""
    version_tuple(tag)
    if os.environ.get('GITHUB_REF') != 'refs/heads/main':
        raise ValueError('Manual releases must run from main')
    latest = github_release('latest')
    if latest and version_tuple(latest['tag_name']) >= version_tuple(tag):
        raise ValueError('Release version must be newer than latest')
    ref = api(f'git/ref/tags/{tag}', missing=True)
    if ref is None:
        commit = git(ROOT, 'rev-parse', 'HEAD')
        subprocess.run(['git', 'merge-base', '--is-ancestor', commit, 'refs/remotes/origin/main'],
                       cwd=ROOT, check=True, capture_output=True)
        api('git/refs', dict(ref='refs/tags/' + tag, sha=commit))
    # Retries reuse the original tag even if main has advanced. Never move a tag.
    git(ROOT, 'fetch', 'origin', f'refs/tags/{tag}:refs/tags/{tag}')
    git(ROOT, 'checkout', '--detach', f'refs/tags/{tag}')
    open_pr(tag)


def open_pr(tag):
    commit = validate_tag(ROOT, tag)
    branch = 'release-notes/' + tag
    existing = api(f'pulls?state=all&head={REPO.split("/")[0]}:{branch}&base=main')
    if existing:
        print(f'Release-note PR already exists: {existing[0]["html_url"]}')
        return
    latest = github_release('latest')
    if latest and version_tuple(latest['tag_name']) >= version_tuple(tag):
        raise ValueError('Release version must be newer than latest')
    data = dict(tag=tag, commit=commit, previous=latest['tag_name'] if latest else None)
    summary = draft(data)
    ref = api(f'git/ref/heads/{branch}', missing=True)
    if ref is None:
        main = api('git/ref/heads/main')['object']['sha']
        api('git/refs', dict(ref='refs/heads/' + branch, sha=main))
    old = api(f'contents/release-notes/{tag}.md?ref={branch}', missing=True)
    # Preserve edits if a previous attempt committed notes but failed to create the PR.
    if old is None:
        write_notes(branch, tag, summary)
    head = api(f'git/ref/heads/{branch}')['object']['sha']
    body = (f'Review `release-notes/{tag}.md` in Files changed (Rendered view).\n\n'
            f'Release source: `{tag}` at `{commit}`. Only these notes are merged; the app builds from the tag.\n\n'
            '- Request changes: `/release-notes revise <instructions>`\n'
            f'- Approve current draft: `/release-notes approve {head}`\n'
            '- Wait for the bot to acknowledge the current commit, then merge manually.\n'
            '- Any subsequent notes commit needs a new approval.\n\n' + marker(MARKER, data))
    pr = api('pulls', dict(title=f'Release notes for {tag}', head=branch, base='main', body=body))
    print(f'Created release-note PR: {pr["html_url"]}')


def command(body):
    body = body.strip()
    match = re.fullmatch(r'/release-notes approve ([a-f0-9]{40})', body)
    if match:
        return 'approve', match[1]
    prefix = '/release-notes revise '
    if body.startswith(prefix) and body[len(prefix):].strip():
        return 'revise', body[len(prefix):].strip()
    raise ValueError('Use /release-notes approve <full-head-sha> or /release-notes revise <instructions>')


def comment(event):
    action, feedback = command(event['comment']['body'])
    number = event['issue']['number']
    permission = api(f'collaborators/{event["comment"]["user"]["login"]}/permission')['permission']
    if permission not in ('admin', 'maintain', 'write'):
        raise ValueError('Only repository writers may request revisions or approve release notes')
    pr = api(f'pulls/{number}')
    if pr['state'] != 'open':
        raise ValueError('Release PR is no longer open')
    data, summary, blob = validate_pr(pr)
    head = pr['head']['sha']
    if action == 'approve' and feedback != head:
        raise ValueError('Approval refers to an older draft; review and approve the current head SHA')
    source = event['comment']['id']
    if action == 'revise':
        api(f'issues/{number}/comments', dict(body='Revision requested; prior approval revoked.\n\n' +
            marker('codex-release-review', dict(action='revise', head=head, source=source))))
        revised = draft(data, summary, feedback)
        current = api(f'pulls/{number}')
        if current['state'] != 'open' or current['head']['sha'] != head:
            raise ValueError('PR changed during generation; retry the revision request')
        if revised != summary:
            result = write_notes(pr['head']['ref'], data['tag'], revised, blob)
            head = result['commit']['sha']
        body = ('Revision request processed. Check Files changed; if generation failed, the existing draft was retained. '
                f'After reviewing, send `/release-notes approve {head}`.\n\n' +
                marker('codex-release-review', dict(action='revise', head=head, source=source)))
    else:
        body = (f'Approved release notes at `{head}`. You may now merge this PR manually.\n\n' +
                marker('codex-release-review', dict(action='approve', head=head, source=source)))
    api(f'issues/{number}/comments', dict(body=body))


def approved_head(comments, head, latest_command):
    state = None
    for entry in comments:
        if entry['user']['id'] != BOT_ID:
            continue
        match = re.search(r'<!-- codex-release-review: (.+?) -->', entry.get('body') or '')
        if match:
            state = json.loads(match[1])
    return state == dict(action='approve', head=head, source=latest_command)


def export_approved(number, output):
    pr = api(f'pulls/{number}')
    if not pr['merged']:
        raise ValueError('Only a merged release-note PR can trigger publication')
    data, summary, _ = validate_pr(pr)
    comments = pages(f'issues/{number}/comments')
    # A newer maintainer command revokes an older acknowledgement even if its
    # workflow is still queued. Outsider comments cannot revoke approval.
    latest_command = None
    latest_action = None
    for entry in comments:
        if entry['user']['id'] == BOT_ID:
            continue
        try:
            parsed_command = command(entry.get('body') or '')
        except ValueError:
            continue
        permission = api(f'collaborators/{entry["user"]["login"]}/permission')['permission']
        if permission in ('admin', 'maintain', 'write'):
            latest_command = entry['id']
            latest_action = parsed_command
    if (latest_action != ('approve', pr['head']['sha'])
            or not approved_head(comments, pr['head']['sha'], latest_command)):
        raise ValueError('Current release-note commit lacks approval; publication stopped')
    latest = github_release('latest')
    if (latest['tag_name'] if latest else None) != data['previous']:
        raise ValueError('Previous release changed during review; draft a new version against latest')
    # Ensure the merge result contains exactly the reviewed notes, including squash/rebase merges.
    merged = api(f'contents/release-notes/{data["tag"]}.md?ref={pr["merge_commit_sha"]}')
    if validate_release_summary(base64.b64decode(merged['content']).decode()) != summary:
        raise ValueError('Merged notes differ from approved draft')
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(summary, encoding='utf-8')
    with open(os.environ['GITHUB_OUTPUT'], 'a') as stream:
        stream.write(f'tag={data["tag"]}\ncommit={data["commit"]}\nprevious={data["previous"] or ""}\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['start', 'open', 'comment', 'export'])
    parser.add_argument('--tag')
    parser.add_argument('--pr', type=int)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if args.action == 'start':
        start_release(args.tag)
    elif args.action == 'open':
        open_pr(args.tag)
    elif args.action == 'comment':
        comment(json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text()))
    else:
        export_approved(args.pr, args.output)


if __name__ == '__main__':
    main()
