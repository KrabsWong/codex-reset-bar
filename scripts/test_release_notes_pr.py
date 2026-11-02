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
        self.summary = '## 新特性\n- 在更新窗口查看版本说明。\n'
        self.pr = dict(number=7, user={'id': review.BOT_ID}, state='open', merged=False,
                       body=review.marker(review.MARKER, self.data),
                       head=dict(ref='release-notes/v0.2.0', sha=self.head, repo={'full_name': release.REPO}),
                       base=dict(ref='main', repo={'full_name': release.REPO}), merge_commit_sha='c' * 40)
        self.event = dict(issue={'number': 7}, comment=dict(id=10, body='/release-notes approve ' + self.head, user={'login': 'maintainer'}))

    def test_commands_are_exact_and_revision_requires_feedback(self):
        self.assertEqual(review.command('/release-notes approve ' + self.head), ('approve', self.head))
        self.assertEqual(review.command('/release-notes revise 请简化描述'), ('revise', '请简化描述'))
        for invalid in ('approve', '/release-notes approve', '/release-notes approve later', '/release-notes revise ', 'text /release-notes approve'):
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

    def test_revision_rechecks_head_before_writing(self):
        self.event['comment']['body'] = '/release-notes revise 简化'
        changed = copy.deepcopy(self.pr)
        changed['head']['sha'] = 'e' * 40
        with patch.object(review, 'api', side_effect=[{'permission': 'write'}, self.pr, {}, changed]), \
             patch.object(review, 'validate_pr', return_value=(self.data, self.summary, 'blob')), \
             patch.object(review, 'draft', return_value='## 优化\n- 更快。\n'), \
             patch.object(review, 'write_notes') as write:
            with self.assertRaisesRegex(ValueError, 'PR changed'):
                review.comment(self.event)
            write.assert_not_called()

    def test_generation_failure_keeps_edits_or_uses_initial_fallback(self):
        with patch.object(review, 'comparison', return_value='diff'), \
             patch.object(review, 'generate', side_effect=ValueError('secret')), \
             contextlib.redirect_stdout(io.StringIO()) as log:
            self.assertEqual(review.draft(self.data), release.DEFAULT_RELEASE_SUMMARY)
            self.assertEqual(review.draft(self.data, self.summary, 'revise'), self.summary)
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
                     {'html_url': 'https://example.invalid/pr/7'}]
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
        self.assertEqual(calls[-1][0][0], 'pulls')
        self.assertIn('/release-notes approve ' + self.head, calls[-1][0][1]['body'])

    def test_existing_pr_is_not_overwritten_on_retry(self):
        with patch.object(review, 'validate_tag', return_value=self.commit), \
             patch.object(review, 'api', return_value=[{'html_url': 'https://example.invalid/pr/7'}]), \
             patch.object(review, 'draft') as draft:
            review.open_pr('v0.2.0')
            draft.assert_not_called()

    def test_possible_credentials_are_not_committed(self):
        with patch.object(review, 'api') as api:
            with self.assertRaisesRegex(ValueError, 'credential'):
                review.write_notes('release-notes/v0.2.0', 'v0.2.0', '## Bugfix\n- ghp_' + 'x' * 30)
            api.assert_not_called()


if __name__ == '__main__':
    unittest.main()
