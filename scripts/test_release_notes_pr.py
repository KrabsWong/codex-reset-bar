import base64
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import release
import release_notes_pr as review


class ReleasePRTests(unittest.TestCase):
    def setUp(self):
        self.head = 'a' * 40
        self.commit = 'b' * 40
        self.data = dict(tag='v0.2.0', commit=self.commit, previous='v0.1.0')
        self.summary = '## New Features\n- View release notes in the update window.\n'
        self.pr = dict(number=7, user={'id': review.BOT_ID}, state='open', merged=False,
                       body=review.marker(review.MARKER, self.data),
                       head=dict(ref='release-notes/v0.2.0', sha=self.head, repo={'full_name': release.REPO}),
                       base=dict(ref='main', repo={'full_name': release.REPO}), merge_commit_sha='c' * 40)
        self.event = dict(issue={'number': 7}, comment=dict(id=10, body='/release-notes approve ' + self.head, user={'login': 'maintainer'}))

    def test_manual_release_creates_tag_and_opens_pr_in_same_run(self):
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/main'}), \
             patch.object(review, 'github_release', return_value={'tag_name': 'v0.1.0'}), \
             patch.object(review, 'api', side_effect=[None, {}]) as api, \
             patch.object(review, 'git', return_value=self.commit) as git, \
             patch.object(review.subprocess, 'run') as run, \
             patch.object(review, 'open_pr') as opened:
            review.start_release('v0.2.0')
        self.assertEqual(api.call_args.args, ('git/refs', {'ref': 'refs/tags/v0.2.0', 'sha': self.commit}))
        self.assertIn('merge-base', run.call_args.args[0])
        self.assertEqual(git.call_args_list[-2].args, (review.ROOT, 'fetch', 'origin', 'refs/tags/v0.2.0:refs/tags/v0.2.0'))
        self.assertEqual(git.call_args.args, (review.ROOT, 'checkout', '--detach', 'refs/tags/v0.2.0'))
        opened.assert_called_once_with('v0.2.0')

    def test_manual_retry_reuses_original_tag_without_creating_or_moving_it(self):
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/main'}), \
             patch.object(review, 'github_release', return_value=None), \
             patch.object(review, 'api', return_value={'object': {'sha': self.commit}}) as api, \
             patch.object(review, 'git') as git, \
             patch.object(review, 'open_pr') as opened:
            review.start_release('v0.2.0')
        api.assert_called_once_with('git/ref/tags/v0.2.0', missing=True)
        self.assertEqual(git.call_count, 2)
        opened.assert_called_once_with('v0.2.0')

    def test_manual_release_rejects_bad_version_branch_and_published_version(self):
        for tag, ref, latest in [('v0.2.0;echo bad', 'refs/heads/main', None),
                                 ('v0.2.0', 'refs/heads/feature/test', None),
                                 ('v0.2.0', 'refs/heads/main', {'tag_name': 'v0.2.0'}),
                                 ('v0.2.0', 'refs/heads/main', {'tag_name': 'v0.3.0'})]:
            with self.subTest(tag=tag, ref=ref, latest=latest), \
                 patch.dict(os.environ, {'GITHUB_REF': ref}), \
                 patch.object(review, 'github_release', return_value=latest), \
                 patch.object(review, 'api') as api, \
                 patch.object(review, 'open_pr') as opened:
                with self.assertRaises(ValueError):
                    review.start_release(tag)
                api.assert_not_called()
                opened.assert_not_called()

    def test_gate_requires_current_authorized_approval_and_bot_acknowledgement(self):
        human = dict(self.event['comment'], user={'id': 123, 'login': 'maintainer'})
        ack = dict(user={'id': review.BOT_ID}, body=review.marker('codex-release-review',
                   dict(action='approve', head=self.head, source=10)))
        for scenario in ('approved', 'no-ack', 'old-head', 'revision', 'opencode', 'deleted', 'edited', 'outsider'):
            comments = [copy.deepcopy(human), ack]
            if scenario == 'no-ack': comments.pop()
            if scenario == 'old-head': comments[0]['body'] = '/release-notes approve ' + 'e' * 40
            if scenario == 'revision': comments.append(dict(human, id=11, body='/release-notes revise Shorten it'))
            if scenario == 'opencode': comments.append(dict(human, id=11, body='/oc Shorten it'))
            if scenario == 'deleted': comments.pop(0)
            if scenario == 'edited': comments[0]['body'] = '/release-notes revise Shorten it'
            permission = 'read' if scenario == 'outsider' else 'write'
            def api(endpoint, *args, **kwargs):
                return {'permission': permission} if endpoint.startswith('collaborators/') else self.pr
            with self.subTest(scenario=scenario), \
                 patch.object(review, 'api', side_effect=api), \
                 patch.object(review, 'pages', side_effect=lambda endpoint: comments if endpoint.endswith('/comments') else []), \
                 patch.object(review, 'validate_pr'), \
                 patch.object(review, 'set_gate') as status:
                review.refresh_gate(7)
                states = [call.args[1] for call in status.call_args_list]
                self.assertEqual(states[0], 'pending')
                self.assertEqual(states[-1], 'success' if scenario == 'approved' else 'pending')

    def test_gate_passes_ordinary_pr_but_fails_closed_for_disguised_notes(self):
        ordinary = copy.deepcopy(self.pr)
        ordinary['head']['ref'] = 'feature/code'
        ordinary['body'] = 'Ordinary code change'
        for files, invalid in [([{'filename': 'src/main.rs'}], False),
                               ([{'filename': 'release-notes/v0.2.0.md'}], True),
                               ([{'filename': 'archive.md', 'previous_filename': 'release-notes/v0.2.0.md'}], True)]:
            with self.subTest(files=files), \
                 patch.object(review, 'api', return_value=ordinary), \
                 patch.object(review, 'pages', return_value=files), \
                 patch.object(review, 'set_gate') as status:
                review.refresh_gate(7)
                self.assertEqual(status.call_args.args[1], 'failure' if invalid else 'success')

    def test_gate_does_not_succeed_if_head_or_latest_command_changes(self):
        for change in ('head', 'command'):
            changed = copy.deepcopy(self.pr)
            if change == 'head': changed['head']['sha'] = 'f' * 40
            with self.subTest(change=change), \
                 patch.object(review, 'api', side_effect=[self.pr, changed]), \
                 patch.object(review, 'pages', return_value=[]), \
                 patch.object(review, 'validate_pr'), \
                 patch.object(review, 'has_approval', side_effect=[True, False]), \
                 patch.object(review, 'set_gate') as status:
                review.refresh_gate(7)
                status.assert_called_once()
                self.assertEqual(status.call_args.args[1], 'pending')

    def test_closed_unmerged_gate_finishes_without_validating_retired_source(self):
        closed = dict(self.pr, state='closed', merged=False)
        with patch.object(review, 'api', return_value=closed), \
             patch.object(review, 'pages') as pages, \
             patch.object(review, 'validate_pr') as validate, \
             patch.object(review, 'has_approval') as approval, \
             patch.object(review, 'set_gate') as status:
            review.refresh_gate(7)
        status.assert_called_once_with(closed, 'error', 'PR closed without merging; no release requested')
        pages.assert_not_called()
        validate.assert_not_called()
        approval.assert_not_called()

    def test_closed_gate_does_not_succeed_after_reopen_or_head_change(self):
        closed = dict(self.pr, state='closed', merged=False)
        for field in ('state', 'merged', 'head', 'base'):
            changed = copy.deepcopy(closed)
            changed[field] = {'state': 'open', 'merged': True,
                              'head': dict(self.pr['head'], sha='f' * 40),
                              'base': dict(self.pr['base'], ref='other')}[field]
            with self.subTest(field=field), \
                 patch.object(review, 'api', side_effect=[closed, changed]), \
                 patch.object(review, 'set_gate') as status:
                review.refresh_gate(7)
            status.assert_not_called()

    def test_merged_gate_does_not_replace_approval_result(self):
        with patch.object(review, 'api', return_value=dict(self.pr, state='closed', merged=True)), \
             patch.object(review, 'set_gate') as status:
            review.refresh_gate(7)
        status.assert_not_called()

    def test_reopened_draft_returns_to_pending_without_approval(self):
        with patch.object(review, 'api', return_value=self.pr), \
             patch.object(review, 'pages', return_value=[]), \
             patch.object(review, 'validate_pr'), \
             patch.object(review, 'has_approval', return_value=False), \
             patch.object(review, 'set_gate') as status:
            review.refresh_gate(7)
        self.assertEqual(status.call_args.args[1], 'pending')

    def test_gate_api_failure_leaves_pending_status(self):
        with patch.object(review, 'api', return_value=self.pr), \
             patch.object(review, 'pages', side_effect=RuntimeError('API unavailable')), \
             patch.object(review, 'set_gate') as status:
            with self.assertRaises(RuntimeError):
                review.refresh_gate(7)
            status.assert_called_once()
            self.assertEqual(status.call_args.args[1], 'pending')

    def test_existing_open_pr_refreshes_gate_on_retry(self):
        pr = dict(self.pr, html_url='https://example.invalid/pr/7')
        with patch.object(review, 'validate_tag', return_value=self.commit), \
             patch.object(review, 'api', return_value=[pr]), \
             patch.object(review, 'refresh_gate') as gate:
            review.open_pr('v0.2.0')
            gate.assert_called_once_with(7)

    def test_commands_are_exact_and_revision_requires_feedback(self):
        self.assertEqual(review.command('/release-notes approve ' + self.head), ('approve', self.head))
        self.assertEqual(review.command('/release-notes revise 请简化描述'), ('revise', '请简化描述'))
        self.assertEqual(review.command('/oc Shorten it'), ('revise', 'Shorten it'))
        self.assertEqual(review.command('/opencode Shorten it'), ('revise', 'Shorten it'))
        for invalid in ('/oc', '/oc ', '/ocean hello', 'approve', '/release-notes approve', '/release-notes approve later', '/release-notes revise ', 'text /release-notes approve'):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                review.command(invalid)

    def test_pr_identity_rejects_forks_other_branches_and_nonbot_authors(self):
        self.assertEqual(review.metadata(self.pr), self.data)
        for change in ('fork', 'branch', 'base', 'author', 'body'):
            pr = copy.deepcopy(self.pr)
            if change == 'fork': pr['head']['repo']['full_name'] = 'other/repo'
            if change == 'branch': pr['head']['ref'] = 'feature/other'
            if change == 'base': pr['base']['ref'] = 'develop'
            if change == 'author': pr['user']['id'] = 123
            if change == 'body': pr['body'] = 'ordinary PR'
            with self.subTest(change=change), self.assertRaises(ValueError):
                review.metadata(pr)

    def test_validation_rejects_moved_tag_or_code_changes(self):
        with patch.object(review, 'git', return_value='d' * 40):
            with self.assertRaisesRegex(ValueError, 'tag moved'):
                review.validate_pr(self.pr)
        with patch.object(review, 'git', return_value=self.commit), \
             patch.object(review, 'pages', return_value=[{'filename': 'src/main.rs', 'status': 'modified'}]):
            with self.assertRaisesRegex(ValueError, 'only its release-note'):
                review.validate_pr(self.pr)

    def test_only_writer_can_approve(self):
        with patch.object(review, 'api', return_value={'permission': 'read'}) as api:
            with self.assertRaisesRegex(ValueError, 'repository writers'):
                review.comment(self.event)
            self.assertEqual(api.call_count, 1)

    def test_approval_is_bound_to_head_and_revision_revokes_it(self):
        approved = dict(user={'id': review.BOT_ID}, body=review.marker('codex-release-review',
                        dict(action='approve', head=self.head, source=10)))
        self.assertTrue(review.approved_head([approved], self.head, 10))
        self.assertFalse(review.approved_head([approved], 'e' * 40, 10))
        forged = dict(approved, user={'id': 123})
        self.assertFalse(review.approved_head([forged], self.head, 10))
        revised = dict(approved, body=review.marker('codex-release-review', dict(action='revise', head=self.head, source=11)))
        self.assertFalse(review.approved_head([approved, revised], self.head, 11))

    def test_approve_posts_exact_current_head(self):
        with patch.object(review, 'api', side_effect=[{'permission': 'write'}, self.pr, {}]) as api, \
             patch.object(review, 'validate_pr', return_value=(self.data, self.summary, 'blob')):
            review.comment(self.event)
        self.assertIn(self.head, api.call_args.args[1]['body'])
        self.assertIn('"action": "approve"', api.call_args.args[1]['body'])

    def test_legacy_revision_does_not_run_custom_generator(self):
        self.event['comment']['body'] = '/release-notes revise Shorten it'
        with patch.object(review, 'api', side_effect=[{'permission': 'write'}, self.pr]), \
             patch.object(review, 'validate_pr', return_value=(self.data, self.summary, 'blob')), \
             patch.object(review, 'draft') as draft:
            with self.assertRaisesRegex(ValueError, 'official OpenCode'):
                review.comment(self.event)
            draft.assert_not_called()

    def test_official_bot_preflight_pins_context_and_edit_scope(self):
        self.event['comment']['body'] = '/oc Explain the benefit'
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'outputs'
            with patch.dict(os.environ, {'GITHUB_OUTPUT': str(output)}), \
                 patch.object(review, 'api', side_effect=[{'permission': 'write'}, self.pr]), \
                 patch.object(review, 'validate_pr', return_value=(self.data, self.summary, 'blob')), \
                 patch.object(review, 'comparison', return_value='fixed tag diff') as comparison:
                review.prepare_opencode(self.event)
            values = dict(line.split('=', 1) for line in output.read_text().splitlines())
            prompt = json.loads(values['prompt'])
            config = json.loads(values['config'])
            self.assertIn('fixed tag diff', prompt)
            self.assertIn('Explain the benefit', prompt)
            self.assertEqual(config['permission']['*'], 'deny')
            self.assertEqual(config['permission']['edit'], {'*': 'deny', 'release-notes/v0.2.0.md': 'allow'})
            comparison.assert_called_once_with(review.ROOT, 'v0.2.0', 'v0.1.0')

    def test_official_bot_rejects_oversized_context_without_truncation(self):
        self.event['comment']['body'] = '/oc Shorten it'
        with patch.object(review, 'api', side_effect=[{'permission': 'write'}, self.pr]), \
             patch.object(review, 'validate_pr', return_value=(self.data, self.summary, 'blob')), \
             patch.object(review, 'comparison', return_value='x' * 120_000):
            with self.assertRaisesRegex(ValueError, '120 KB'):
                review.prepare_opencode(self.event)

    def test_official_bot_rejects_outsiders_before_preparing_context(self):
        self.event['comment']['body'] = '/oc Shorten it'
        with patch.object(review, 'api', return_value={'permission': 'read'}), \
             patch.object(review, 'comparison') as comparison:
            with self.assertRaisesRegex(ValueError, 'repository writers'):
                review.prepare_opencode(self.event)
            comparison.assert_not_called()

    def test_generation_failure_uses_initial_fallback(self):
        with patch.object(review, 'comparison', return_value='diff'), \
             patch.object(review, 'generate', side_effect=ValueError('secret')), \
             contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(review.draft(self.data), release.DEFAULT_RELEASE_SUMMARY)
        self.assertNotIn('secret', log.getvalue())

    def test_merged_unapproved_draft_cannot_publish(self):
        self.pr['merged'] = True
        with patch.object(review, 'api', return_value=self.pr), \
             patch.object(review, 'validate_pr', return_value=(self.data, self.summary, 'blob')), \
             patch.object(review, 'pages', return_value=[]):
            with self.assertRaisesRegex(ValueError, 'lacks approval'):
                review.export_approved(7, Path('/unused'))

    def test_export_approved_merged_notes_pins_original_release_commit(self):
        self.pr['merged'] = True
        content = {'content': base64.b64encode(self.summary.encode()).decode()}
        approval = {'user': {'id': review.BOT_ID}, 'body': review.marker('codex-release-review',
                    dict(action='approve', head=self.head, source=10))}
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'summary.md'
            metadata = Path(directory) / 'outputs'
            with patch.dict(os.environ, {'GITHUB_OUTPUT': str(metadata)}), \
                 patch.object(review, 'api', side_effect=[self.pr, {'permission': 'write'}, content]), \
                 patch.object(review, 'validate_pr', return_value=(self.data, self.summary, 'blob')), \
                 patch.object(review, 'pages', return_value=[dict(self.event['comment'], user={'id': 123, 'login': 'maintainer'}), approval]), \
                 patch.object(review, 'github_release', return_value={'tag_name': 'v0.1.0'}):
                review.export_approved(7, output)
            self.assertEqual(output.read_text(), self.summary)
            self.assertIn(f'commit={self.commit}', metadata.read_text())
            with patch.dict(os.environ, {'RELEASE_SUMMARY_FILE': str(output)}):
                self.assertEqual(release.load_release_summary('v0.2.0'), self.summary)

    def test_stale_approval_command_cannot_approve_new_draft(self):
        self.event['comment']['body'] = '/release-notes approve ' + 'e' * 40
        with patch.object(review, 'api', side_effect=[{'permission': 'write'}, self.pr]) as api, \
             patch.object(review, 'validate_pr', return_value=(self.data, self.summary, 'blob')):
            with self.assertRaisesRegex(ValueError, 'older draft'):
                review.comment(self.event)
            self.assertEqual(api.call_count, 2)

    def test_pending_revision_prevents_export_before_bot_processes_it(self):
        self.pr['merged'] = True
        approved = dict(user={'id': review.BOT_ID}, body=review.marker('codex-release-review',
                        dict(action='approve', head=self.head, source=10)))
        revision = dict(id=11, user={'id': 123, 'login': 'maintainer'}, body='/release-notes revise 修改第一条')
        with patch.object(review, 'api', side_effect=[self.pr, {'permission': 'write'}]), \
             patch.object(review, 'validate_pr', return_value=(self.data, self.summary, 'blob')), \
             patch.object(review, 'pages', return_value=[approved, revision]):
            with self.assertRaisesRegex(ValueError, 'lacks approval'):
                review.export_approved(7, Path('/unused'))

    def test_edited_approval_comment_cannot_keep_old_approval(self):
        self.pr['merged'] = True
        approved = dict(user={'id': review.BOT_ID}, body=review.marker('codex-release-review',
                        dict(action='approve', head=self.head, source=10)))
        edited = dict(id=10, user={'id': 123, 'login': 'maintainer'}, body='/release-notes revise 修改第一条')
        with patch.object(review, 'api', side_effect=[self.pr, {'permission': 'write'}]), \
             patch.object(review, 'validate_pr', return_value=(self.data, self.summary, 'blob')), \
             patch.object(review, 'pages', return_value=[edited, approved]):
            with self.assertRaisesRegex(ValueError, 'lacks approval'):
                review.export_approved(7, Path('/unused'))

    def test_changed_baseline_or_merge_result_stops_publication(self):
        self.pr['merged'] = True
        approval = {'user': {'id': review.BOT_ID}, 'body': review.marker('codex-release-review',
                    dict(action='approve', head=self.head, source=10))}
        human = dict(self.event['comment'], user={'id': 123, 'login': 'maintainer'})
        for scenario in ('baseline', 'merge'):
            with self.subTest(scenario=scenario), \
                 patch.object(review, 'api', side_effect=[self.pr, {'permission': 'write'}, {
                     'content': base64.b64encode(b'Fixed some known issues.').decode()}]), \
                 patch.object(review, 'validate_pr', return_value=(self.data, self.summary, 'blob')), \
                 patch.object(review, 'pages', return_value=[human, approval]), \
                 patch.object(review, 'github_release', return_value={
                     'tag_name': 'v0.1.1' if scenario == 'baseline' else 'v0.1.0'}):
                with self.assertRaisesRegex(ValueError, 'Previous release changed|Merged notes differ'):
                    review.export_approved(7, Path('/unused'))

    def test_open_creates_only_notes_branch_and_pr(self):
        calls = []
        responses = [[], None, {'object': {'sha': self.commit}}, {}, None,
                     {'commit': {'sha': self.head}}, {'object': {'sha': self.head}},
                     dict(self.pr, html_url='https://example.invalid/pr/7'), {}]
        def api(*args, **kwargs):
            calls.append((args, kwargs))
            return responses.pop(0)
        with patch.object(review, 'validate_tag', return_value=self.commit), \
             patch.object(review, 'github_release', return_value={'tag_name': 'v0.1.0'}), \
             patch.object(review, 'draft', return_value=self.summary), \
             patch.object(review, 'api', side_effect=api):
            review.open_pr('v0.2.0')
        write = calls[5][0]
        self.assertEqual(write[0], 'contents/release-notes/v0.2.0.md')
        self.assertEqual(base64.b64decode(write[1]['content']).decode(), self.summary)
        self.assertIn('\n\n', write[1]['message'])
        self.assertEqual(calls[-2][0][0], 'pulls')
        self.assertIn('/release-notes approve ' + self.head, calls[-2][0][1]['body'])
        self.assertEqual(calls[-1][0][0], 'statuses/' + self.head)
        self.assertEqual(calls[-1][0][1]['state'], 'pending')

    def test_closed_unmerged_pr_can_be_replaced_after_branch_cleanup(self):
        closed = dict(self.pr, state='closed', merged=False, merged_at=None)
        new_pr = dict(self.pr, number=8, html_url='https://example.invalid/pr/8')
        responses = [[closed], None, None, {'object': {'sha': self.commit}}, {}, None,
                     {'commit': {'sha': self.head}}, {'object': {'sha': self.head}}, new_pr, {}]
        with patch.object(review, 'validate_tag', return_value=self.commit), \
             patch.object(review, 'github_release', side_effect=[None, {'tag_name': 'v0.1.0'}]), \
             patch.object(review, 'draft', return_value=self.summary) as draft, \
             patch.object(review, 'api', side_effect=responses) as api:
            review.open_pr('v0.2.0')
        draft.assert_called_once_with(self.data)
        self.assertEqual(api.call_args_list[-2].args[0], 'pulls')
        self.assertIn('"commit": "' + self.commit, api.call_args_list[-2].args[1]['body'])
        self.assertEqual(api.call_args.args[1]['state'], 'pending')
        self.assertTrue(api.call_args.args[1]['target_url'].endswith('/8'))

    def test_replacement_rejects_merged_published_or_uncleaned_drafts(self):
        for scenario in ('merged', 'published', 'branch'):
            closed = dict(self.pr, state='closed', merged_at='2026-01-01' if scenario == 'merged' else None)
            responses = [[closed], {'object': {'sha': self.head}}]
            with self.subTest(scenario=scenario), \
                 patch.object(review, 'validate_tag', return_value=self.commit), \
                 patch.object(review, 'github_release', return_value={'tag_name': 'v0.2.0'} if scenario == 'published' else None), \
                 patch.object(review, 'api', side_effect=responses), \
                 patch.object(review, 'draft') as draft:
                with self.assertRaisesRegex(ValueError, 'already merged|already exists|old branch'):
                    review.open_pr('v0.2.0')
                draft.assert_not_called()

    def test_existing_pr_is_not_overwritten_on_retry(self):
        with patch.object(review, 'validate_tag', return_value=self.commit), \
             patch.object(review, 'api', return_value=[dict(self.pr, html_url='https://example.invalid/pr/7')]), \
             patch.object(review, 'refresh_gate'), \
             patch.object(review, 'draft') as draft:
            review.open_pr('v0.2.0')
            draft.assert_not_called()

    def test_possible_credentials_are_not_committed(self):
        with patch.object(review, 'api') as api:
            with self.assertRaisesRegex(ValueError, 'credential'):
                review.write_notes('release-notes/v0.2.0', 'v0.2.0', '## Bug Fixes\n- ghp_' + 'x' * 30)
            api.assert_not_called()


if __name__ == '__main__':
    unittest.main()
