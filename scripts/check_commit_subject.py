#!/usr/bin/env python3
"""Validate a Conventional Commit subject for a commit or squash PR title."""

import argparse
import re
import subprocess
import sys
import unicodedata


HEADER_PATTERN = (
    r'(feat|fix|docs|style|refactor|perf|test|chore)'
    r'(\([a-z0-9][a-z0-9._/-]*\))?!?: \S[^\r\n]*'
)


def valid_subject(subject):
    return (
        not any(unicodedata.category(char) in {'Cc', 'Zl', 'Zp'} for char in subject)
        and re.fullmatch(HEADER_PATTERN, subject) is not None
    )


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--subject', help='Subject to check; default: the current Git commit')
    args = parser.parse_args(argv)
    subject = args.subject
    if subject is None:
        try:
            result = subprocess.run(
                ['git', 'log', '-1', '--format=%s'], check=True,
                capture_output=True, text=True, timeout=5,
            )
            subject = result.stdout.rstrip('\n')
        except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired):
            print('Cannot read the current Git commit subject.', file=sys.stderr)
            return 1
    if not valid_subject(subject):
        print(
            'Use a Conventional Commit subject: type(scope): description. '
            'Types: feat, fix, docs, style, refactor, perf, test, chore. '
            'Scope is optional; ! marks a breaking change. '
            'Use one line without control characters.',
            file=sys.stderr,
        )
        return 1
    print('Conventional Commit subject: passed')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
