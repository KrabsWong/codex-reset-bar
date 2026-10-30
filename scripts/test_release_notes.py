import subprocess
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch

import release
from release import generate_release_notes


class ReleaseNotesTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git('init', '-b', 'main')

    def git(self, *args):
        return subprocess.check_output(
            ['git', '-c', 'user.name=Release Test', '-c', 'user.email=release@example.invalid', *args],
            cwd=self.root, text=True, stderr=subprocess.DEVNULL).strip()

    def commit(self, subject):
        self.git('commit', '--allow-empty', '-m', subject)
        return self.git('rev-parse', 'HEAD')

    def test_first_release_lists_all_commits_and_full_ids(self):
        first = self.commit('Initial app')
        second = self.commit('Add updater')
        self.git('tag', '-a', 'v0.1.0', '-m', 'First release')
        notes = generate_release_notes('v0.1.0', None, self.root)
        self.assertIn('First published release', notes)
        self.assertIn('Commits (2)', notes)
        self.assertIn(f'Initial app — [{first}]', notes)
        self.assertIn(f'Add updater — [{second}]', notes)
        self.assertLess(notes.index('Initial app'), notes.index('Add updater'))
        self.assertEqual(notes, generate_release_notes('v0.1.0', None, self.root))

    def test_incremental_release_includes_merged_commits_and_excludes_baseline(self):
        baseline = self.commit('Old release')
        self.git('tag', '-a', 'v0.1.0', '-m', 'Baseline')
        self.git('checkout', '-b', 'feature')
        feature = self.commit('Fix update dialog')
        self.git('checkout', 'main')
        main = self.commit('Improve packaging')
        self.git('merge', '--no-ff', 'feature', '-m', 'Merge update work')
        merge = self.git('rev-parse', 'HEAD')
        self.git('tag', 'v0.2.0')
        notes = generate_release_notes('v0.2.0', 'v0.1.0', self.root)
        self.assertNotIn(baseline, notes)
        self.assertNotIn('Old release', notes)
        for commit in [feature, main, merge]:
            self.assertIn(f'[{commit}]', notes)
        self.assertIn('Commits (3)', notes)
        self.assertIn('/compare/v0.1.0...v0.2.0', notes)

    def test_rejects_non_ancestor_previous_release(self):
        self.commit('Base')
        self.git('checkout', '-b', 'other')
        self.commit('Unmerged work')
        self.git('tag', 'v0.1.0')
        self.git('checkout', 'main')
        self.commit('Current work')
        self.git('tag', 'v0.2.0')
        with self.assertRaisesRegex(ValueError, 'not an ancestor'):
            generate_release_notes('v0.2.0', 'v0.1.0', self.root)

    def test_commit_subjects_cannot_inject_markdown_html_or_mentions(self):
        self.commit('Fix [link](https://bad.invalid) <script> @someone *bold* `code` & text')
        self.git('tag', 'v0.1.0')
        notes = generate_release_notes('v0.1.0', None, self.root)
        self.assertIn(r'\[link\]\(https://bad\.invalid\)', notes)
        self.assertIn('&lt;script&gt;', notes)
        self.assertIn('&#64;someone', notes)
        self.assertIn(r'\*bold\*', notes)
        self.assertIn(r'\`code\` &amp; text', notes)
        self.assertNotIn('<script>', notes)

    def test_publish_passes_notes_file_before_upload_and_updates_existing_draft(self):
        self.commit('Initial app')
        self.git('tag', 'v0.1.0')
        self.commit('Fix updater')
        self.git('tag', 'v0.2.0')
        (self.root / 'dist').mkdir()
        (self.root / 'dist/Codex-Resets-0.2.0-macOS-arm64.zip').touch()
        (self.root / 'dist/appcast.xml').touch()
        original_run = release.run
        for draft in [None, {'draft': True}]:
            calls = []
            def capture(*args, **kwargs):
                if args[0] == 'gh':
                    calls.append(args)
                    return None
                return original_run(*args, **kwargs)
            with patch.object(release, 'ROOT', self.root), \
                 patch.object(release, 'github_release', side_effect=[{'tag_name': 'v0.1.0'}, draft]), \
                 patch.object(release, 'run', side_effect=capture):
                release.publish('v0.2.0')
            self.assertEqual(calls[0][2], 'edit' if draft else 'create')
            self.assertIn('--notes-file', calls[0])
            self.assertNotIn('--generate-notes', calls[0])
            self.assertIn('--draft=false', calls[-1])
            notes = (self.root / 'dist/release-notes.md').read_text()
            self.assertIn('Fix updater', notes)
            self.assertNotIn('Initial app', notes)


if __name__ == '__main__':
    unittest.main()
