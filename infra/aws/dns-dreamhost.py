#!/usr/bin/env python3
"""Add only a demo CNAME and its ACM validation records; never remove existing DNS."""
import argparse
import json
import os
from pathlib import Path
import sys
import urllib.parse
import urllib.request
import uuid


def normalize(value):
    return value.lower().rstrip('.')


LOWER = 'abcdefghijklmnopqrstuvwxyz'
ALNUM = LOWER + '0123456789'
HEX = '0123456789abcdef'


def dns_label(value, alphabet=ALNUM + '-'):
    return (1 <= len(value) <= 63 and value[0] in ALNUM and value[-1] in ALNUM
            and all(character in alphabet for character in value))


def acm_label(value):
    return (2 <= len(value) <= 63 and value[0] == '_'
            and all(character in HEX for character in value[1:]))


def planned_records(domain, validation, target):
    labels = domain.split('.') if len(domain) <= 253 else []
    if (len(labels) < 2 or not all(dns_label(label) for label in labels)
            or len(labels[-1]) < 2 or not all(character in LOWER for character in labels[-1])):
        raise ValueError('Use a full lowercase DNS hostname')
    records = []
    for record in validation:
        name, value = normalize(record['name']), normalize(record['value'])
        prefix, _, hostname = name.partition('.')
        parts = value.split('.') if len(value) <= 253 else []
        if (record['type'] != 'CNAME' or len(name) > 253
                or not acm_label(prefix) or hostname != domain
                or len(parts) != 4 or not acm_label(parts[0])
                or not dns_label(parts[1], ALNUM) or parts[2:] != ['acm-validations', 'aws']):
            raise ValueError('Validation record must be an ACM CNAME for this exact hostname')
        records.append({'record': name, 'type': 'CNAME', 'value': value})
    if target:
        parts = target.split('.') if len(target) <= 253 else []
        if (len(parts) != 3 or parts[1:] != ['cloudfront', 'net']
                or not parts[0].startswith('d') or len(parts[0]) < 2
                or not dns_label(parts[0], ALNUM)):
            raise ValueError('Target must be a generated CloudFront hostname')
        records.append({'record': domain, 'type': 'CNAME', 'value': target})
    if not records:
        raise ValueError('Supply --validation-file or --target')
    return records


def api(command, **fields):
    query = urllib.parse.urlencode({'key': os.environ['DREAMHOST_API_KEY'], 'cmd': command,
                                    'format': 'json', 'unique_id': str(uuid.uuid4()), **fields})
    try:
        with urllib.request.urlopen('https://api.dreamhost.com/?' + query, timeout=30) as response:
            result = json.load(response)
    except Exception as error:
        raise RuntimeError('DreamHost request failed: ' + type(error).__name__) from None
    if result.get('result') != 'success':
        raise RuntimeError('DreamHost API rejected the DNS operation')
    return result['data']


def apply_records(records):
    existing = api('dns-list_records')
    pending = []
    # Validate every requested name before adding anything. Never replace another record.
    for record in records:
        matches = [r for r in existing if normalize(r['record']) == record['record']]
        if matches and not all(r['type'] == 'CNAME' and normalize(r['value']) == record['value'] for r in matches):
            raise ValueError('Existing DNS conflicts with requested CNAME: ' + record['record'])
        if not matches:
            pending.append(record)
    for record in pending:
        api('dns-add_record', **record, comment='Weather Bridge certificate or CloudFront endpoint')
        print('Added ' + record['record'])
    print('DNS records are present; allow time for authoritative propagation.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--domain', required=True)
    parser.add_argument('--validation-file', type=Path, help='JSON from Terraform certificate_dns_validation_records')
    parser.add_argument('--target', help='Generated CloudFront hostname, without https://')
    parser.add_argument('--apply', action='store_true', help='Add absent records; otherwise print the plan only')
    args = parser.parse_args()
    validation = json.loads(args.validation_file.read_text()) if args.validation_file else []
    records = planned_records(args.domain, validation, args.target)
    print(json.dumps(records, indent=2))
    if not args.apply:
        return
    if not os.environ.get('DREAMHOST_API_KEY'):
        raise ValueError('Inject DREAMHOST_API_KEY through your credential manager')
    apply_records(records)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, RuntimeError, KeyError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
