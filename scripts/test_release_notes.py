import subprocess
import tempfile
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET
from unittest.mock import patch

import release
from release import generate_release_notes


class ReleaseNotesTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.git('init', '-b', 'main')
        (self.root / 'release-notes').mkdir()
        self.summary = '## 新特性\n- 在更新弹窗中查看本次版本的更新摘要。\n\n## Bugfix\n- 修复含有 & 和 < 字符的内容显示。\n'
        for tag in ('v0.1.0', 'v0.2.0'):
            (self.root / 'release-notes' / f'{tag}.md').write_text(self.summary)

    def write_feed(self, summary=None):
        feed = ET.Element('rss')
        item = ET.SubElement(ET.SubElement(feed, 'channel'), 'item')
        ET.SubElement(item, f'{{{release.SPARKLE}}}version').text = '0.2.0'
        release.add_release_summary(item, 'v0.2.0', self.summary if summary is None else summary)
        ET.ElementTree(feed).write(self.root / 'dist/appcast.xml', encoding='utf-8')

    def git(self, *args):
        return subprocess.check_output(
            ['git', '-c', 'user.name=Release Test', '-c', 'user.email=release@example.invalid', *args],
            cwd=self.root, text=True, stderr=subprocess.DEVNULL).strip()

    def commit(self, subject):
        self.git('commit', '--allow-empty', '-m', subject)
        return self.git('rev-parse', 'HEAD')

    def test_summary_requires_real_categories_and_items(self):
        path = self.root / 'release-notes/v0.2.0.md'
        for invalid in ('## 新特性', '## 新特性\n- TODO: describe change',
                        '## Bugfix\n- 待填写', '## Other\n- Change',
                        '## 优化\n- One\n## 优化\n- Two',
                        '## 新特性\n## Bugfix\n- Fix',
                        '## Bugfix\n- Fix\n## 优化', 'Uncategorized change'):
            with self.subTest(summary=invalid):
                path.write_text(invalid)
                with self.assertRaises(ValueError):
                    release.load_release_summary('v0.2.0', self.root)
        with self.assertRaises(ValueError):
            release.load_release_summary('../../outside', self.root)

    def test_single_category_is_enough_and_template_is_rejected(self):
        path = self.root / 'release-notes/v0.2.0.md'
        path.write_text('## 优化\n- 缩短打开设置窗口的等待时间。\n')
        self.assertEqual(release.load_release_summary('v0.2.0', self.root), path.read_text())
        path.write_text((Path(__file__).resolve().parents[1] / 'release-notes/TEMPLATE.md').read_text())
        with self.assertRaisesRegex(ValueError, 'placeholder'):
            release.load_release_summary('v0.2.0', self.root)

    def test_embedded_markdown_round_trips_xml_special_characters(self):
        (self.root / 'dist').mkdir()
        summary = self.summary + '- 保留 **重点**、中文、<tag>、& 和 ]]>。\n'
        self.write_feed(summary)
        item = ET.parse(self.root / 'dist/appcast.xml').find('./channel/item')
        description = item.find('description')
        self.assertEqual(description.get(f'{{{release.SPARKLE}}}format'), 'markdown')
        self.assertIn(summary, description.text)
        self.assertIn('/releases/tag/v0.2.0', description.text)
        self.assertEqual(item.findtext(f'{{{release.SPARKLE}}}fullReleaseNotesLink'),
                         f'https://github.com/{release.REPO}/releases/tag/v0.2.0')

    def test_publish_rejects_stale_summary_before_github_mutation(self):
        (self.root / 'dist').mkdir()
        (self.root / 'dist/Codex-Resets-0.2.0-macOS-arm64.zip').touch()
        self.write_feed('## 优化\n- Outdated summary.\n')
        with patch.object(release, 'ROOT', self.root), patch.object(release, 'github_release') as github:
            with self.assertRaisesRegex(ValueError, 'prepare and sign again'):
                release.publish('v0.2.0')
            github.assert_not_called()

    def test_publish_rejects_missing_notes_wrong_format_and_wrong_version(self):
        (self.root / 'dist').mkdir()
        (self.root / 'dist/Codex-Resets-0.2.0-macOS-arm64.zip').touch()
        for invalid in ('missing', 'format', 'version'):
            with self.subTest(invalid=invalid):
                self.write_feed()
                path = self.root / 'dist/appcast.xml'
                tree = ET.parse(path)
                item = tree.find('./channel/item')
                if invalid == 'missing':
                    item.remove(item.find('description'))
                elif invalid == 'format':
                    item.find('description').set(f'{{{release.SPARKLE}}}format', 'html')
                else:
                    item.find(f'{{{release.SPARKLE}}}version').text = '0.1.0'
                tree.write(path)
                with patch.object(release, 'ROOT', self.root), patch.object(release, 'github_release') as github:
                    with self.assertRaisesRegex(ValueError, 'prepare and sign again'):
                        release.publish('v0.2.0')
                    github.assert_not_called()

    def test_unavailable_summary_uses_fixed_english_fallback(self):
        path = self.root / 'release-notes/v0.2.0.md'
        path.unlink()
        self.assertEqual(release.load_release_summary('v0.2.0', self.root), 'Fixed some known issues.\n')
        for content in (b'', b' \n\t', b'\xff'):
            path.write_bytes(content)
            self.assertEqual(release.load_release_summary('v0.2.0', self.root), release.DEFAULT_RELEASE_SUMMARY)
        with patch.object(Path, 'read_text', side_effect=PermissionError):
            self.assertEqual(release.load_release_summary('v0.2.0', self.root), release.DEFAULT_RELEASE_SUMMARY)

    def test_fallback_is_shared_by_feed_and_github_release_with_commit_history(self):
        (self.root / 'release-notes/v0.2.0.md').unlink()
        commit = self.commit('Actual change')
        self.git('tag', 'v0.2.0')
        (self.root / 'dist').mkdir()
        summary = release.load_release_summary('v0.2.0', self.root)
        self.write_feed(summary)
        (self.root / 'dist/Codex-Resets-0.2.0-macOS-arm64.zip').touch()
        original_run = release.run
        def capture(*args, **kwargs):
            if args[0] != 'gh':
                return original_run(*args, **kwargs)
        with patch.object(release, 'ROOT', self.root), \
             patch.object(release, 'github_release', return_value=None), \
             patch.object(release, 'run', side_effect=capture):
            release.publish('v0.2.0')
        notes = (self.root / 'dist/release-notes.md').read_text()
        description = ET.parse(self.root / 'dist/appcast.xml').findtext('./channel/item/description')
        self.assertIn('Fixed some known issues.', notes)
        self.assertIn('Fixed some known issues.', description)
        self.assertIn(f'Actual change — [{commit}]', notes)

    def test_first_release_lists_all_commits_and_full_ids(self):
        first = self.commit('Initial app')
        second = self.commit('Add updater')
        self.git('tag', '-a', 'v0.1.0', '-m', 'First release')
        notes = generate_release_notes('v0.1.0', None, self.root)
        self.assertIn(self.summary.strip(), notes)
        self.assertLess(notes.index('## 新特性'), notes.index('## Commits'))
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
        self.write_feed()
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
            self.assertIn(self.summary.strip(), notes)
            self.assertIn('Fix updater', notes)
            self.assertNotIn('Initial app', notes)


if __name__ == '__main__':
    unittest.main()
