import base64
import copy
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import start_version_release as version
from release_notes_pr import BOT_ID, marker


class VersionTests(unittest.TestCase):
    def test_semantic_increments(self):
        for level, expected in [('patch', 'v0.1.11'), ('minor', 'v0.2.0'), ('major', 'v1.0.0')]:
            self.assertEqual(version.next_tag('v0.1.10', level), expected)
        self.assertEqual(version.next_tag('v0.0.0', 'patch'), 'v0.0.1')
        with self.assertRaises(ValueError):
            version.next_tag('v1.2.3-rc1', 'patch')

    def test_version_transform_keeps_dependencies_and_cargo_accepts_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'src').mkdir()
            (root / 'src/main.rs').write_text('fn main() {}')
            (root / 'Cargo.toml').write_text('[package]\nname = "codex-reset"\nversion = "0.1.4"\nedition = "2021"\n')
            subprocess.run(['cargo', 'generate-lockfile', '--offline'], cwd=root, check=True, capture_output=True)
            subprocess.run(['git', 'init', '-q'], cwd=root, check=True)
            def git(*args):
                return subprocess.check_output(['git', *args], cwd=root, text=True)
            git('add', '.')
            git('-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '-qm', 'Fixture\n\nInitial fixture')
            with patch.object(version, 'ROOT', root):
                files = version.version_files('HEAD', 'v0.1.11')
            for path, text in files.items():
                (root / path).write_text(text)
            subprocess.run(['cargo', 'metadata', '--locked', '--offline', '--no-deps', '--format-version=1'], cwd=root, check=True, capture_output=True)
            self.assertIn('version = "0.1.11"', files['Cargo.toml'])
            self.assertIn('version = "0.1.11"', files['Cargo.lock'])

    def test_version_pr_refuses_unexpected_files_content_or_author(self):
        data = dict(base='a' * 40, tag='v0.1.11', previous='v0.1.10')
        pr = dict(number=9, user={'id': BOT_ID}, body=marker('codex-version', data),
                  base={'ref': 'main', 'repo': {'full_name': version.REPO}},
                  head={'ref': 'release-version/v0.1.11', 'sha': 'b'*40, 'repo': {'full_name': version.REPO}})
        for case in ('valid', 'author', 'files', 'content'):
            current = copy.deepcopy(pr)
            if case == 'author': current['user']['id'] = 123
            files = [{'filename': f, 'status': 'modified'} for f in version.FILES]
            if case == 'files': files.append({'filename': 'src/main.rs', 'status': 'modified'})
            with self.subTest(case=case), patch.object(version, 'pages', return_value=files), \
                 patch.object(version, 'version_files', return_value={f: 'expected' for f in version.FILES}), \
                 patch.object(version, 'api', side_effect=lambda endpoint: {'parents': [{'sha': data['base']}]} if endpoint.startswith('git/commits/') else {'content': base64.b64encode(b'wrong' if case == 'content' else b'expected').decode()}), \
                 patch('subprocess.run'):
                if case == 'valid': version.validate_version_pr(current, data)
                else:
                    with self.assertRaises(ValueError): version.validate_version_pr(current, data)

    def test_unfinished_notes_prevent_new_release(self):
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/main', 'GITHUB_RUN_ID': '123'}), \
             patch.object(version, 'github_release', return_value={'tag_name': 'v0.1.10'}), \
             patch.object(version, 'pages', return_value=[{'head': {'ref': 'release-notes/v0.1.11'}}]), \
             patch.object(version, 'api') as api:
            with self.assertRaisesRegex(ValueError, 'existing release-note'):
                version.start('patch')
            api.assert_not_called()

    def test_manual_start_requires_main(self):
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/feature/test'}), patch.object(version, 'api') as api:
            with self.assertRaises(ValueError): version.start('patch')
            api.assert_not_called()

    def test_create_merge_tag_and_notes_run_in_order_and_resume_merged_pr(self):
        for resume, request, advanced in ((False, False, False), (True, False, False),
                                           (False, True, False), (True, True, False),
                                           (False, True, True), (True, True, True)):
            base, head, merged = 'a'*40, 'b'*40, 'c'*40
            data = dict(tag='v0.1.11', previous='v0.1.10', base=base, run='123')
            if request: data['request'] = 'release/patch/test-1'
            pr = dict(number=9, state='closed' if resume else 'open', merged=resume,
                      merge_commit_sha=merged, mergeable_state='clean',
                      body=marker('codex-version', data), head={'sha': head})
            calls = []
            def api(endpoint, payload=None, method=None, **kwargs):
                calls.append((endpoint, payload, method))
                if endpoint == 'git/matching-refs/tags/v': return []
                if endpoint.startswith('pulls?'): return [pr] if resume else []
                if endpoint == 'git/ref/tags/v0.1.11': return {'object': {'sha': merged}} if resume else None
                if endpoint == 'git/ref/heads/main': return {'object': {'sha': base}}
                if endpoint == 'git/commits/' + merged: return {'parents': [{'sha': 'f'*40 if advanced else base}, {'sha': head}]}
                if endpoint == 'git/commits/' + base: return {'tree': {'sha': 'd'*40}}
                if endpoint == 'git/trees': return {'sha': 'e'*40}
                if endpoint == 'git/commits': return {'sha': head}
                if endpoint in ('pulls', 'pulls/9'): return pr
                if endpoint == 'pulls/9/merge': return {'merged': True, 'sha': merged}
                return {}
            with self.subTest(resume=resume, request=request, advanced=advanced), patch.dict(os.environ, {'GITHUB_REF': 'refs/tags/release/patch/test-1' if request else 'refs/heads/main', 'GITHUB_RUN_ID': '123'}), \
                 patch.object(version, 'github_release', return_value={'tag_name': 'v0.1.10'}), \
                 patch.object(version, 'pages', return_value=[]), \
                 patch.object(version, 'api', side_effect=api), \
                 patch.object(version, 'git', side_effect=lambda root, *args: base if args[0] == 'rev-parse' else 'expected') as git, \
                 patch('subprocess.run'), \
                 patch.object(version, 'version_files', return_value={f: 'expected\n' for f in version.FILES}), \
                 patch.object(version, 'validate_version_pr', return_value={f: 'expected\n' for f in version.FILES}), \
                 patch.object(version, 'refresh_gate') as gate, \
                 patch.object(version, 'open_pr') as notes:
                if advanced:
                    with self.assertRaisesRegex(ValueError, 'outside the release request'):
                        version.start(request_tag='release/patch/test-1')
                    notes.assert_not_called()
                    self.assertFalse(any(c[0] == 'git/refs' and c[1]['ref'].startswith('refs/tags/') for c in calls))
                    continue
                version.start(request_tag='release/patch/test-1') if request else version.start('patch')
            notes.assert_called_once_with('v0.1.11')
            self.assertEqual(git.call_args.args, (version.ROOT, 'checkout', '--detach', 'v0.1.11'))
            if resume:
                gate.assert_not_called()
                self.assertFalse(any(c[0] == 'git/refs' for c in calls))
            else:
                gate.assert_called_once_with(9)
                merge_index = next(i for i, c in enumerate(calls) if c[0] == 'pulls/9/merge')
                tag_index = next(i for i, c in enumerate(calls) if c[0] == 'git/refs' and c[1]['ref'].startswith('refs/tags/'))
                self.assertLess(merge_index, tag_index)
                self.assertEqual(calls[merge_index][1]['sha'], head)
                self.assertEqual(calls[tag_index][1]['sha'], merged)

    def test_existing_unmanaged_tag_cannot_be_replaced(self):
        def api(endpoint, *args, **kwargs):
            if endpoint == 'git/matching-refs/tags/v' or endpoint.startswith('pulls?'): return []
            return {'object': {'sha': 'a'*40}}
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/main', 'GITHUB_RUN_ID': '123'}), \
             patch.object(version, 'github_release', return_value={'tag_name': 'v0.1.10'}), \
             patch.object(version, 'pages', return_value=[]), patch.object(version, 'api', side_effect=api), \
             patch.object(version, 'open_pr') as notes:
            with self.assertRaisesRegex(ValueError, 'already exists'): version.start('patch')
            notes.assert_not_called()

    def test_published_run_retry_does_not_increment_again(self):
        saved = dict(tag='v0.1.11', previous='v0.1.10', base='a'*40, run='123')
        old = dict(user={'id': BOT_ID}, body=marker('codex-version', saved))
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/main', 'GITHUB_RUN_ID': '123'}), \
             patch.object(version, 'github_release', return_value={'tag_name': 'v0.1.11'}), \
             patch.object(version, 'pages', return_value=[old]), \
             patch.object(version, 'api') as api, patch.object(version, 'open_pr') as notes:
            version.start('patch')
        api.assert_not_called()
        notes.assert_not_called()

    def test_rejects_tampered_baseline_and_new_manifest_changes(self):
        data = dict(base='a'*40, tag='v0.1.11', previous='v0.1.10')
        pr = dict(number=9, user={'id': BOT_ID}, body=marker('codex-version', data),
                  base={'ref': 'main', 'repo': {'full_name': version.REPO}},
                  head={'ref': 'release-version/v0.1.11', 'sha': 'b'*40, 'repo': {'full_name': version.REPO}})
        for scenario in ('parent', 'main-manifest'):
            with self.subTest(scenario=scenario), \
                 patch.object(version, 'pages', return_value=[{'filename': p, 'status': 'modified'} for p in version.FILES]), \
                 patch.object(version, 'api', return_value={'parents': [{'sha': 'f'*40 if scenario == 'parent' else data['base']}]}), \
                 patch.object(version, 'version_files', side_effect=[{'Cargo.toml': 'original'}, {'Cargo.toml': 'dependency changed'}]), \
                 patch('subprocess.run'):
                with self.assertRaisesRegex(ValueError, 'single generated commit|Main version files changed'):
                    version.validate_version_pr(pr, data)

    def test_filtered_pagination_preserves_query(self):
        import release_notes_pr
        with patch.object(release_notes_pr, 'api', return_value=[]) as api:
            release_notes_pr.pages('pulls?state=all&base=main')
        api.assert_called_once_with('pulls?state=all&base=main&per_page=100&page=1')

    def test_merged_version_retry_uses_original_files_not_later_main(self):
        data = dict(base='a'*40, tag='v0.1.11', previous='v0.1.10')
        pr = dict(number=9, merged=True, user={'id': BOT_ID}, body=marker('codex-version', data),
                  base={'ref': 'main', 'repo': {'full_name': version.REPO}},
                  head={'ref': 'release-version/v0.1.11', 'sha': 'b'*40, 'repo': {'full_name': version.REPO}})
        expected = {p: 'original versioned content' for p in version.FILES}
        with patch.object(version, 'pages', return_value=[{'filename': p, 'status': 'modified'} for p in version.FILES]), \
             patch.object(version, 'api', side_effect=lambda endpoint: {'parents': [{'sha': data['base']}]} if endpoint.startswith('git/commits/') else {'content': base64.b64encode(b'original versioned content').decode()}), \
             patch.object(version, 'version_files', side_effect=lambda base, tag: expected if base == data['base'] else {'Cargo.toml': 'later main'}) as files, \
             patch('subprocess.run'):
            self.assertEqual(version.validate_version_pr(pr, data), expected)
        files.assert_called_once_with(data['base'], data['tag'])

    def test_request_tag_level_and_invalid_names(self):
        for level in ('patch', 'minor', 'major'):
            self.assertEqual(version.request_level(f'release/{level}/20261102-1'), level)
        for name in ('v0.1.11', 'release/patch', 'release/patch/', 'release/other/1', 'release/patch/a/b', 'release/patch/a;echo'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                version.request_level(name)

    def test_request_tag_must_match_event_and_be_on_main(self):
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/tags/release/minor/test'}), patch.object(version, 'api') as api:
            with self.assertRaisesRegex(ValueError, 'triggering tag'):
                version.start(request_tag='release/patch/test')
            api.assert_not_called()
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/tags/release/patch/test'}), \
             patch.object(version, 'git', return_value='a'*40), \
             patch('subprocess.run', side_effect=subprocess.CalledProcessError(1, 'git')), \
             patch.object(version, 'api') as api:
            with self.assertRaises(subprocess.CalledProcessError):
                version.start(request_tag='release/patch/test')
            api.assert_not_called()

    def test_stale_main_request_tag_stops_before_creating_version(self):
        def api(endpoint, *args, **kwargs):
            if endpoint == 'git/matching-refs/tags/v' or endpoint.startswith('pulls?'): return []
            if endpoint == 'git/ref/tags/v0.1.11': return None
            if endpoint == 'git/ref/heads/main': return {'object': {'sha': 'b'*40}}
            self.fail('Unexpected mutation: ' + endpoint)
        with patch.dict(os.environ, {'GITHUB_REF': 'refs/tags/release/patch/test', 'GITHUB_RUN_ID': '123'}), \
             patch.object(version, 'git', return_value='a'*40), patch('subprocess.run'), \
             patch.object(version, 'github_release', return_value={'tag_name': 'v0.1.10'}), \
             patch.object(version, 'pages', return_value=[]), patch.object(version, 'api', side_effect=api):
            with self.assertRaisesRegex(ValueError, 'current main'):
                version.start(request_tag='release/patch/test')
