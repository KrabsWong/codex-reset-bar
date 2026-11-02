"""Regression checks for release entry points and the actual workflow shell route."""
import ast
import fnmatch
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest


WORKFLOW = Path(__file__).resolve().parents[1] / '.github/workflows/release.yml'


class ReleaseWorkflowTests(unittest.TestCase):
    def test_push_filters_accept_only_release_request_tags(self):
        text = WORKFLOW.read_text()
        filters = ast.literal_eval(re.search(r'  push:\n    tags: (\[.*\])', text)[1])
        for tag in ('release/patch/test-1', 'release/minor/test-2', 'release/major/test-3',
                    'v0.1.12', 'v1.0.0', 'v2.0.0-rc1', 'main', 'release/other/test'):
            with self.subTest(tag=tag):
                self.assertEqual(any(fnmatch.fnmatchcase(tag, pattern) for pattern in filters),
                                 tag.startswith(('release/patch/', 'release/minor/', 'release/major/')))

    def route(self, event, tag='', level=''):
        step = WORKFLOW.read_text().split('      - name: Generate notes and open review PR\n', 1)[1]
        block = step.split('        run: |\n', 1)[1].split('\n\n', 1)[0]
        script = '\n'.join(line[10:] for line in block.splitlines())
        with tempfile.TemporaryDirectory() as directory:
            # Capture the requested command; do not invoke GitHub or create real tags.
            python = Path(directory) / 'python3'
            python.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
            python.chmod(0o755)
            return subprocess.run(['bash', '-e', '-c', script], text=True, capture_output=True,
                                  env=dict(os.environ, PATH=directory + os.pathsep + os.environ['PATH'],
                                           GITHUB_EVENT_NAME=event, RELEASE_TAG=tag, RELEASE_LEVEL=level))

    def test_request_tags_and_manual_dispatch_use_version_preparation(self):
        for level in ('patch', 'minor', 'major'):
            tag = f'release/{level}/test-1'
            result = self.route('push', tag)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.splitlines(), ['scripts/start_version_release.py', '--request-tag', tag])
            result = self.route('workflow_dispatch', 'main', level)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.splitlines(), ['scripts/start_version_release.py', '--level', level])

    def test_formal_tag_and_unexpected_event_cannot_fall_back_to_notes_generation(self):
        for event, tag in [('push', 'v0.1.12'), ('push', 'main'), ('pull_request', 'release/patch/test')]:
            with self.subTest(event=event, tag=tag):
                result = self.route(event, tag)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, '')
