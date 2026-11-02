import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import generate_release_summary as generator
import release


class SummaryGenerationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.summary = '## New Features\n- View release notes directly in the update window.\n'

    def git(self, *args):
        return subprocess.check_output(['git', '-c', 'user.name=Test', '-c',
                                       'user.email=test@example.invalid', *args],
                                      cwd=self.root, text=True, stderr=subprocess.DEVNULL).strip()

    def test_comparison_uses_actual_diff_and_correct_release_range(self):
        self.git('init', '-b', 'main')
        (self.root / 'app.txt').write_text('Old behavior\n')
        self.git('add', '.')
        self.git('commit', '-m', 'Baseline')
        self.git('tag', 'v0.1.0')
        (self.root / 'app.txt').write_text('New user-facing behavior\n')
        self.git('commit', '-am', 'Show update summary')
        self.git('tag', 'v0.2.0')
        context = generator.comparison(self.root, 'v0.2.0', 'v0.1.0')
        self.assertIn('+New user-facing behavior', context)
        self.assertIn('-Old behavior', context)
        self.assertNotIn('Baseline', context)
        first = generator.comparison(self.root, 'v0.2.0', None)
        self.assertIn('Baseline', first)
        self.assertIn('+New user-facing behavior', first)
        with self.assertRaises(subprocess.CalledProcessError):
            generator.comparison(self.root, 'v0.1.0', 'v0.2.0')

    def test_json_events_extract_only_valid_final_text(self):
        output = '\n'.join(json.dumps(event) for event in [
            {'type': 'step_start'}, {'type': 'reasoning', 'part': {'text': 'internal'}},
            {'type': 'text', 'part': {'text': self.summary}}, {'type': 'step_finish'}])
        self.assertEqual(generator.parse_output(output), self.summary)
        for output in ('', '{}', 'not json', '{"type":"error"}',
                       json.dumps({'type': 'text', 'part': {'text': '```markdown\n' + self.summary + '```'}})):
            with self.subTest(output=output), self.assertRaises(ValueError):
                generator.parse_output(output)
        self.assertEqual(generator.parse_output(json.dumps({'type': 'text', 'part': {
            'text': release.DEFAULT_RELEASE_SUMMARY}})), release.DEFAULT_RELEASE_SUMMARY)

    def test_agent_receives_context_without_github_or_signing_credentials(self):
        def execute(command, **kwargs):
            self.assertEqual(command[:4], ['opencode', 'run', '--format', 'json'])
            self.assertIn('actual diff', kwargs['input'])
            self.assertNotIn('GH_TOKEN', kwargs['env'])
            self.assertNotIn('SPARKLE_PRIVATE_ED_KEY', kwargs['env'])
            self.assertEqual(kwargs['timeout'], 180)
            config = json.loads((kwargs['cwd'] / 'opencode.json').read_text())
            self.assertEqual(config['permission'], {'*': 'deny'})
            return subprocess.CompletedProcess(command, 0, stdout=json.dumps({
                'type': 'text', 'part': {'text': self.summary}}))
        with patch.dict(os.environ, {'GH_TOKEN': 'fixture', 'SPARKLE_PRIVATE_ED_KEY': 'fixture'}), \
             patch.object(generator.subprocess, 'run', side_effect=execute):
            self.assertEqual(generator.generate('actual diff', 'opencode-go/example', 'fixture'), self.summary)


if __name__ == '__main__':
    unittest.main()
