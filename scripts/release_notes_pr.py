#!/usr/bin/env python3
"""Create release-note PRs and authorize a merged, explicitly approved draft."""
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
APPROVAL_STATUS = 'Release notes approval'


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
        status = re.search(r'HTTP (\d{3})', result.stderr)
        detail = f' (HTTP {status[1]})' if status else ''
        raise RuntimeError(f'GitHub API request failed{detail}; no publication authorized')
    return json.loads(result.stdout) if result.stdout.strip() else None


def pages(endpoint):
    result = []
    for page in range(1, 101):
        separator = '&' if '?' in endpoint else '?'
        rows = api(f'{endpoint}{separator}per_page=100&page={page}')
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


def draft(data):
    try:
        context = comparison(ROOT, data['tag'], data['previous'])
        return generate(context, os.environ.get('OPENCODE_MODEL', ''),
                        os.environ.get('OPENCODE_GO_API_KEY', ''))
    except (OSError, ValueError, RuntimeError, KeyError, TypeError, subprocess.SubprocessError):
        print('::warning::AI generation unavailable; using fixed fallback.')
        return DEFAULT_RELEASE_SUMMARY


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
    active = next((pr for pr in existing if pr['state'] == 'open'), None)
    if active:
        refresh_gate(active['number'])
        print(f'Release-note PR already exists: {active["html_url"]}')
        return
    if any(pr.get('merged_at') or pr.get('merged') for pr in existing):
        raise ValueError('Release notes were already merged; use a new version')
    if existing:
        if github_release('tags/' + tag):
            raise ValueError('Release already exists; use a new version')
        if api(f'git/ref/heads/{branch}', missing=True) is not None:
            raise ValueError('Closed release-note PR still has its branch; remove the old branch before restarting')
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
            '- Request changes: `/oc <instructions>` (official OpenCode bot)\n'
            f'- Approve current draft: `/release-notes approve {head}`\n'
            '- Wait for the bot to acknowledge the current commit, then merge manually.\n'
            '- Any subsequent notes commit needs a new approval.\n\n' + marker(MARKER, data))
    pr = api('pulls', dict(title=f'Release notes for {tag}', head=branch, base='main', body=body))
    set_gate(pr, 'pending', 'Review the notes and approve the current commit before merging')
    print(f'Created release-note PR: {pr["html_url"]}')


def command(body):
    body = body.strip()
    match = re.fullmatch(r'/release-notes approve ([a-f0-9]{40})', body)
    if match:
        return 'approve', match[1]
    for prefix in ('/oc ', '/opencode '):
        if body.startswith(prefix) and body[len(prefix):].strip():
            return 'revise', body[len(prefix):].strip()
    # Old requests still revoke approval, but no longer execute the custom reviser.
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
        raise ValueError('Post /oc <instructions> to use the official OpenCode bot')
    body = (f'Approved release notes at `{head}`. You may now merge this PR manually.\n\n' +
            marker('codex-release-review', dict(action='approve', head=head, source=source)))
    api(f'issues/{number}/comments', dict(body=body))


def prepare_opencode(event):
    """Validate a writer's request before allowing the official bot to run."""
    action, feedback = command(event['comment']['body'])
    if action != 'revise' or not event['comment']['body'].strip().startswith(('/oc ', '/opencode ')):
        raise ValueError('Use /oc <instructions>')
    login = event['comment']['user']['login']
    if api(f'collaborators/{login}/permission')['permission'] not in ('admin', 'maintain', 'write'):
        raise ValueError('Only repository writers may request revisions')
    pr = api(f'pulls/{event["issue"]["number"]}')
    if pr['state'] != 'open':
        raise ValueError('Release PR is no longer open')
    data, summary, _ = validate_pr(pr)
    from generate_release_summary import PROMPT
    instructions = PROMPT.replace('Return ONLY Markdown with these headings',
                                  'Edit the release-note file using these headings')
    prompt = ('The following formatting instructions apply to the file content, not your reply.\n' + instructions + '\nModify only release-notes/' + data['tag'] + '.md. '
              'Do not approve, merge, publish, change branches, or execute shell commands. '
              'The official runner handles committing and pushing. Explain the result in your reply.\n'
              + comparison(ROOT, data['tag'], data['previous'])
              + '\nCurrent draft:\n' + summary + '\nMaintainer request:\n' + feedback)
    # Linux limits each environment string; fail explicitly rather than truncate.
    if len(prompt.encode('utf-8')) > 120_000:
        raise ValueError('Revision context exceeds 120 KB; edit the note manually in GitHub')
    config = dict(permission={'*': 'deny', 'read': {'*': 'allow', '.git/*': 'deny', '**/.git/**': 'deny'},
                              'edit': {'*': 'deny', f'release-notes/{data["tag"]}.md': 'allow'}},
                  provider={'opencode-go': {'options': {'apiKey': '{env:OPENCODE_GO_API_KEY}'}}})
    # JSON produces single-line outputs; no user text is interpolated into shell code.
    with open(os.environ['GITHUB_OUTPUT'], 'a') as stream:
        stream.write('prompt=' + json.dumps(prompt) + '\n')
        stream.write('config=' + json.dumps(config) + '\n')
        stream.write('tag=' + data['tag'] + '\n')


def approved_head(comments, head, latest_command):
    state = None
    for entry in comments:
        if entry['user']['id'] != BOT_ID:
            continue
        match = re.search(r'<!-- codex-release-review: (.+?) -->', entry.get('body') or '')
        if match:
            state = json.loads(match[1])
    return state == dict(action='approve', head=head, source=latest_command)


def has_approval(pr):
    comments = pages(f'issues/{pr["number"]}/comments')
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
    return (latest_action == ('approve', pr['head']['sha'])
            and approved_head(comments, pr['head']['sha'], latest_command))


def set_gate(pr, state, description):
    api(f'statuses/{pr["head"]["sha"]}', dict(
        state=state, context=APPROVAL_STATUS, description=description,
        target_url=f'https://github.com/{REPO}/pull/{pr["number"]}'))


def refresh_gate(number):
    pr = api(f'pulls/{number}')
    if pr['state'] != 'open' or pr['base']['ref'] != 'main':
        return
    set_gate(pr, 'pending', 'Checking release-note approval')
    files = pages(f'pulls/{number}/files')
    release_pr = (pr['head']['ref'].startswith('release-notes/')
                  or f'<!-- {MARKER}:' in (pr.get('body') or '')
                  or any(re.fullmatch(r'release-notes/v[^/]+\.md', entry.get(key, ''))
                         for entry in files for key in ('filename', 'previous_filename')))
    if release_pr:
        try:
            validate_pr(pr)
        except (ValueError, KeyError, TypeError):
            set_gate(pr, 'failure', 'Invalid release-note PR; check its identity and changed files')
            return
        if not has_approval(pr):
            set_gate(pr, 'pending', 'Waiting for approval of the current release-note commit')
            return
    # Never transfer approval to a head that changed while this job was running.
    current = api(f'pulls/{number}')
    if (current['state'] != 'open' or current['head'] != pr['head']
            or current['base'] != pr['base'] or current.get('body') != pr.get('body')):
        return
    if release_pr and not has_approval(current):
        return
    set_gate(pr, 'success', 'Current release notes approved' if release_pr else 'No release-note approval required')


def export_approved(number, output):
    pr = api(f'pulls/{number}')
    if not pr['merged']:
        raise ValueError('Only a merged release-note PR can trigger publication')
    data, summary, _ = validate_pr(pr)
    if not has_approval(pr):
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
    parser.add_argument('action', choices=['start', 'open', 'comment', 'export', 'gate', 'prepare-opencode'])
    parser.add_argument('--tag')
    parser.add_argument('--pr', type=int)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if args.action == 'start':
        start_release(args.tag)
    elif args.action == 'open':
        open_pr(args.tag)
    elif args.action == 'gate':
        refresh_gate(args.pr)
    elif args.action == 'prepare-opencode':
        prepare_opencode(json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text()))
    elif args.action == 'comment':
        comment(json.loads(Path(os.environ['GITHUB_EVENT_PATH']).read_text()))
    else:
        export_approved(args.pr, args.output)


if __name__ == '__main__':
    main()
