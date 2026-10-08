"""DNS changes must stay inside the selected hostname and preserve existing records."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('dreamhost_dns', Path(__file__).with_name('dns-dreamhost.py'))
dns = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dns)
DOMAIN = 'weather.example.com'
VALIDATION = {'name': '_abcd.' + DOMAIN + '.', 'type': 'CNAME',
              'value': '_ef01.validation.acm-validations.aws.'}


class DnsTests(unittest.TestCase):
    def test_expected_names_are_normalized(self):
        records = dns.planned_records(DOMAIN, [VALIDATION], 'd123.cloudfront.net')
        self.assertEqual([r['record'] for r in records], ['_abcd.' + DOMAIN, DOMAIN])
        self.assertEqual(records[0]['value'], '_ef01.validation.acm-validations.aws')

    def test_foreign_validation_name_is_rejected(self):
        with self.assertRaises(ValueError):
            dns.planned_records(DOMAIN, [{**VALIDATION, 'name': '_abcd.example.com'}], None)

    def test_foreign_validation_target_is_rejected(self):
        with self.assertRaises(ValueError):
            dns.planned_records(DOMAIN, [{**VALIDATION, 'value': 'attacker.example.com'}], None)

    def test_non_cloudfront_destination_is_rejected(self):
        with self.assertRaises(ValueError):
            dns.planned_records(DOMAIN, [], 'attacker.example.com')

    def test_existing_matching_record_is_not_added_again(self):
        records = dns.planned_records(DOMAIN, [], 'd123.cloudfront.net')
        with patch.object(dns, 'api', return_value=[{**records[0], 'value': 'd123.cloudfront.net.'}]) as api:
            dns.apply_records(records)
        api.assert_called_once_with('dns-list_records')

    def test_all_conflicts_are_checked_before_any_addition(self):
        records = dns.planned_records(DOMAIN, [VALIDATION], 'd123.cloudfront.net')
        with patch.object(dns, 'api', return_value=[{'record': DOMAIN, 'type': 'A', 'value': '192.0.2.1'}]) as api:
            with self.assertRaises(ValueError):
                dns.apply_records(records)
        api.assert_called_once_with('dns-list_records')

    def test_only_absent_selected_record_is_added(self):
        records = dns.planned_records(DOMAIN, [], 'd123.cloudfront.net')
        with patch.object(dns, 'api', side_effect=[[], 'record_added']) as api:
            dns.apply_records(records)
        self.assertEqual(api.call_args_list[1].args, ('dns-add_record',))
        self.assertEqual(api.call_args_list[1].kwargs['record'], DOMAIN)


if __name__ == '__main__':
    unittest.main()
