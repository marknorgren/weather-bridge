"""Behavior checks for Conventional Commit subjects and squash PR titles."""

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from scripts import check_commit_subject


class CommitSubjectTests(unittest.TestCase):
    def test_repository_types_and_optional_scope_and_breaking_marker(self):
        for subject in (
            'feat: add hourly forecasts',
            'fix(api): bound requests',
            'docs: explain deployment',
            'style(web): format controls',
            'refactor(weather): share parsing',
            'perf(cache): coalesce requests',
            'test(cli): cover ambiguity',
            'chore(ci): check commit titles',
            'feat!: change the public contract',
            'fix(api/v1)!: reject an unsafe input',
            'docs: preserve Unicode café and 東京',
            'feat: preserve emoji 👩‍💻 in descriptions',
        ):
            with self.subTest(subject=subject):
                self.assertTrue(check_commit_subject.valid_subject(subject))

    def test_malformed_or_multiline_titles_fail(self):
        for subject in (
            '', 'Add a feature', 'Merge pull request #1',
            'Fix: repair input', 'build: change the toolchain',
            'fix(): repair input', 'fix(api: repair input',
            'fix(api)!!: repair input', 'fix:repair input',
            'fix: ', 'fix:  ', ' fix: repair input',
            'fix: repair\nchore: forge another subject',
            'fix: repair\r', 'fix: repair\x1b[31m',
            'fix: repair\tinput', 'fix: repair\x7f',
            'fix: repair\x85chore: another subject',
            'fix: repair\x9b31m',
            'fix: repair\u2028chore: another subject',
            'fix: repair\u2029chore: another subject',
        ):
            with self.subTest(subject=repr(subject)):
                self.assertFalse(check_commit_subject.valid_subject(subject))

    def test_cli_treats_title_as_data_and_does_not_log_the_payload(self):
        script = Path(__file__).with_name('check_commit_subject.py')
        for subject, expected in (
            ('fix: handle $(exit 42) and `exit 42` as text', 0),
            ('--help', 1),
            ('fix: input\n::error::forged annotation', 1),
        ):
            with self.subTest(subject=repr(subject)):
                result = subprocess.run(
                    [sys.executable, str(script), '--subject=' + subject],
                    capture_output=True, text=True, timeout=5,
                )
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertNotIn(subject, result.stdout + result.stderr)
                if expected:
                    self.assertIn('Conventional Commit', result.stderr)

    def test_default_checks_the_actual_git_commit(self):
        script = Path(__file__).with_name('check_commit_subject.py').resolve()
        with tempfile.TemporaryDirectory() as directory:
            subprocess.run(['git', 'init', '--quiet', directory], check=True)
            for subject, expected in (
                ('chore(ci): enforce Conventional Commits', 0),
                ('An invalid squash title', 1),
            ):
                with self.subTest(subject=subject):
                    subprocess.run(
                        ['git', '-c', 'core.hooksPath=/dev/null',
                         '-c', 'commit.gpgsign=false', '-c', 'user.name=Test',
                         '-c', 'user.email=test@example.invalid', 'commit',
                         '--quiet', '--allow-empty', '-m', subject],
                        cwd=directory, check=True, timeout=5,
                    )
                    result = subprocess.run(
                        [sys.executable, str(script)], cwd=directory,
                        capture_output=True, text=True, timeout=5,
                    )
                    self.assertEqual(result.returncode, expected, result.stderr)

    def test_default_fails_when_there_is_no_git_commit(self):
        script = Path(__file__).with_name('check_commit_subject.py').resolve()
        with tempfile.TemporaryDirectory() as directory:
            subprocess.run(['git', 'init', '--quiet', directory], check=True)
            result = subprocess.run(
                [sys.executable, str(script)], cwd=directory,
                capture_output=True, text=True, timeout=5,
            )
        self.assertEqual(result.returncode, 1)
        self.assertIn('Cannot read', result.stderr)


if __name__ == '__main__':
    unittest.main()
