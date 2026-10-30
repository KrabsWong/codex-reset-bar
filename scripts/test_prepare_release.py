import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from prepare_release import prepare, update_manifest, validate_tag


class PrepareReleaseTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.git('init', '-b', 'main')
        self.git('config', 'user.name', 'Release Test')
        self.git('config', 'user.email', 'release-test@example.invalid')
        (self.root / 'src').mkdir()
        (self.root / 'src/main.rs').write_text('fn main() {}\n')
        (self.root / 'Cargo.toml').write_text('[package]\nname = "codex-reset"\nversion = "0.1.4"\nedition = "2024"\n\n[dependencies]\n')
        subprocess.run(['cargo', 'generate-lockfile', '--offline'], cwd=self.root, check=True, capture_output=True)
        self.commit('Initial package')
        self.git('update-ref', 'refs/remotes/origin/main', 'HEAD')

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root, text=True, stderr=subprocess.DEVNULL).strip()

    def commit(self, title):
        self.git('add', '.')
        self.git('commit', '-m', title, '-m', 'Create isolated release-validation fixture.')

    def test_syncs_major_minor_patch_tags_without_moving_refs(self):
        before = self.git('rev-parse', 'HEAD')
        for version in ('0.1.5', '0.2.0', '1.0.0'):
            tag = 'v' + version
            self.git('tag', '-a', tag, '-m', 'Release fixture')
            prepare(self.root, tag)
            metadata = json.loads(subprocess.check_output(
                ['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=self.root, text=True))
            self.assertEqual(metadata['packages'][0]['version'], version)
            self.assertIn(f'version = "{version}"', (self.root / 'Cargo.lock').read_text())
            self.assertEqual(self.git('rev-parse', 'HEAD'), before)
            self.assertEqual(self.git('rev-parse', tag + '^{commit}'), before)

    def test_unmerged_branch_is_rejected_before_version_changes(self):
        original = (self.root / 'Cargo.toml').read_bytes()
        self.git('checkout', '-b', 'feature/unfinished')
        (self.root / 'feature.txt').write_text('unfinished')
        self.commit('Unmerged feature')
        self.git('tag', 'v0.1.5')
        with self.assertRaisesRegex(ValueError, 'merged into'):
            prepare(self.root, 'v0.1.5')
        self.assertEqual((self.root / 'Cargo.toml').read_bytes(), original)

    def test_merged_commit_is_accepted_even_when_main_advanced(self):
        self.git('tag', 'v0.1.5')
        tagged = self.git('rev-parse', 'HEAD')
        (self.root / 'later.txt').write_text('later')
        self.commit('Later main commit')
        self.git('update-ref', 'refs/remotes/origin/main', 'HEAD')
        self.git('checkout', '--detach', tagged)
        self.assertEqual(validate_tag(self.root, 'v0.1.5'), tagged)

    def test_tag_head_mismatch_is_rejected(self):
        self.git('tag', 'v0.1.5')
        (self.root / 'later.txt').write_text('later')
        self.commit('Different checkout')
        with self.assertRaisesRegex(ValueError, 'checked-out'):
            validate_tag(self.root, 'v0.1.5')

    def test_only_package_version_is_changed(self):
        text = '[package]\nname = "codex-reset"\nversion = "0.1.4" # keep comment\n[dependencies.example]\nversion = "7.0.0"\n'
        updated = update_manifest(text, '1.0.0')
        self.assertIn('version = "1.0.0" # keep comment', updated)
        self.assertIn('version = "7.0.0"', updated)

    def test_rejects_invalid_tag_and_missing_main(self):
        with self.assertRaises(ValueError):
            validate_tag(self.root, 'patch-1')
        self.git('tag', 'v0.1.5')
        with self.assertRaises(ValueError):
            validate_tag(self.root, 'v0.1.5', 'refs/remotes/origin/missing')


if __name__ == '__main__':
    unittest.main()
