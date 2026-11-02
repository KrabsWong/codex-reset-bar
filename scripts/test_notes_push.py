import tempfile
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

import check_notes_push as guard


class NotesPushTests(unittest.TestCase):
    def test_scans_intermediate_history_and_restricts_files(self):
        for scenario in ('valid', 'secret-removed', 'other-file'):
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as directory:
                def git(*args):
                    return subprocess.check_output(['git', '-C', directory, *args], text=True).strip()
                git('init', '-q')
                git('config', 'user.email', 'test@example.invalid')
                git('config', 'user.name', 'Test')
                note = Path(directory) / 'release-notes/v0.1.10.md'
                note.parent.mkdir()
                note.write_text('Initial notes')
                git('add', '.')
                git('commit', '-qm', 'Initial\n\nTest fixture')
                base = git('rev-parse', 'HEAD')
                if scenario == 'secret-removed':
                    note.write_text('ghp_' + 'x' * 30)
                    git('add', '.')
                    git('commit', '-qm', 'Intermediate\n\nTest fixture')
                if scenario == 'other-file':
                    (Path(directory) / 'other.txt').write_text('unexpected')
                note.write_text('Updated notes')
                git('add', '.')
                git('commit', '-qm', 'Final\n\nTest fixture')
                head = git('rev-parse', 'HEAD')
                line = f'refs/heads/release-notes/v0.1.10 {head} refs/heads/release-notes/v0.1.10 {base}'
                real_run = subprocess.run
                def run(args, **kwargs):
                    return real_run(args, cwd=directory, **kwargs)
                with patch.object(guard, 'git', side_effect=git), patch.object(guard.subprocess, 'run', side_effect=run):
                    if scenario == 'valid':
                        guard.check([line], 'v0.1.10')
                    else:
                        with self.assertRaises(ValueError):
                            guard.check([line], 'v0.1.10')

    def test_rejects_other_destination_and_branch_creation(self):
        for ref, remote in [('refs/heads/main', 'b' * 40), ('refs/heads/release-notes/v0.1.10', '0' * 40)]:
            with self.assertRaises(ValueError):
                guard.check([f'HEAD {"a" * 40} {ref} {remote}'], 'v0.1.10')
