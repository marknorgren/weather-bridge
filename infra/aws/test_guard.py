import importlib.util
import os
from pathlib import Path
import unittest
from unittest.mock import patch, call

os.environ.update(TARGET_FUNCTION='weather-bridge-demo', TARGET_REGION='us-east-1', RESUME_CONCURRENCY='3',
                  DISTRIBUTION_ID='E2EXAMPLE')
spec = importlib.util.spec_from_file_location('guard', Path(__file__).with_name('guard.py'))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

BELOW = {'requests': 1, 'computeEstimateGBSeconds': 1, 'cloudFrontRequests': 1, 'cloudFrontBytes': 1}


@patch.object(guard, 'set_distribution_enabled')
@patch.object(guard, 'set_concurrency')
@patch.object(guard, 'collect_usage')
class GuardTests(unittest.TestCase):
    def test_lambda_thresholds(self, collect, concurrency, distribution):
        self.assertFalse(guard.breached({**BELOW, 'requests': 799999, 'computeEstimateGBSeconds': 319999}))
        self.assertTrue(guard.breached({**BELOW, 'requests': 800000}))
        self.assertTrue(guard.breached({**BELOW, 'computeEstimateGBSeconds': 320000}))

    def test_cloudfront_thresholds_are_80_percent_of_the_always_free_tier(self, collect, concurrency, distribution):
        self.assertFalse(guard.breached({**BELOW, 'cloudFrontRequests': 7999999,
                                         'cloudFrontBytes': 799999999999}))
        self.assertTrue(guard.breached({**BELOW, 'cloudFrontRequests': 8000000}))
        self.assertTrue(guard.breached({**BELOW, 'cloudFrontBytes': 800 * 10 ** 9}))

    def test_failed_read_stops_lambda_and_cloudfront_and_reports_failure(self, collect, concurrency, distribution):
        collect.side_effect = PermissionError('missing metrics permission')
        with self.assertRaises(PermissionError):
            guard.handler({}, None)
        concurrency.assert_called_once_with(0)
        distribution.assert_called_once_with(False)

    def test_breach_cannot_be_resumed(self, collect, concurrency, distribution):
        collect.return_value = {**BELOW, 'requests': 800000}
        result = guard.handler({'action': 'resume'}, None)
        self.assertTrue(result['stopped'])
        concurrency.assert_called_once_with(0)
        distribution.assert_called_once_with(False)

    def test_cloudfront_breach_disables_the_distribution(self, collect, concurrency, distribution):
        collect.return_value = {**BELOW, 'cloudFrontRequests': 9000000}
        result = guard.handler({'action': 'check'}, None)
        self.assertTrue(result['stopped'])
        concurrency.assert_called_once_with(0)
        distribution.assert_called_once_with(False)

    def test_health_or_billing_signal_stops_without_waiting_for_metrics(self, collect, concurrency, distribution):
        guard.handler({'Records': [{'EventSource': 'aws:sns'}]}, None)
        concurrency.assert_called_once_with(0)
        distribution.assert_called_once_with(False)
        collect.assert_not_called()

    def test_lambda_stops_even_if_disabling_cloudfront_fails(self, collect, concurrency, distribution):
        distribution.side_effect = RuntimeError('distribution busy')
        with self.assertRaises(RuntimeError):
            guard.handler({'action': 'stop'}, None)
        concurrency.assert_called_once_with(0)

    @patch.object(guard, 'client')
    def test_resume_restores_concurrency_and_distribution(self, client, collect, concurrency, distribution):
        collect.return_value = BELOW
        client.return_value.get_function_concurrency.return_value = {'ReservedConcurrentExecutions': 3}
        distribution.return_value = True
        result = guard.handler({'action': 'resume'}, None)
        concurrency.assert_called_once_with(3)
        distribution.assert_called_once_with(True)
        self.assertFalse(result['stopped'])
        self.assertEqual(result['resumeConcurrency'], 3)

    def test_failed_distribution_resume_stops_lambda_again(self, collect, concurrency, distribution):
        collect.return_value = BELOW
        distribution.side_effect = [PermissionError('update denied'), False]
        with self.assertRaises(PermissionError):
            guard.handler({'action': 'resume'}, None)
        self.assertEqual(concurrency.call_args_list, [call(3), call(0)])
        self.assertEqual(distribution.call_args_list, [call(True), call(False)])

    @patch.object(guard, 'client')
    def test_failed_resume_verification_stops_demo(self, client, collect, concurrency, distribution):
        collect.return_value = BELOW
        client.return_value.get_function_concurrency.side_effect = PermissionError('read denied')
        with self.assertRaises(PermissionError):
            guard.handler({'action': 'resume'}, None)
        self.assertEqual(concurrency.call_args_list, [call(3), call(0)])
        self.assertEqual(distribution.call_args_list, [call(True), call(False)])

    @patch.object(guard, 'client')
    def test_check_reports_a_disabled_distribution_as_stopped(self, client, collect, concurrency, distribution):
        collect.return_value = BELOW
        client.return_value.get_function_concurrency.return_value = {'ReservedConcurrentExecutions': 3}
        with patch.object(guard, 'distribution_enabled', return_value=False):
            result = guard.handler({'action': 'check'}, None)
        concurrency.assert_not_called()
        distribution.assert_not_called()
        self.assertTrue(result['stopped'])
        self.assertFalse(result['distributionEnabled'])

    def test_resume_concurrency_is_bounded(self, collect, concurrency, distribution):
        self.assertEqual(guard.resume_concurrency('3'), 3)
        for bad in ['0', '11', '-1', 'three', '']:
            with self.assertRaises(ValueError):
                guard.resume_concurrency(bad)


class FakeCloudFront:
    def __init__(self, enabled, distributions=('E2EXAMPLE',)):
        self.config = {'CallerReference': 'x', 'Enabled': enabled, 'Comment': 'demo'}
        self.distributions = distributions
        self.updates = []

    def get_distribution_config(self, Id):
        assert Id == 'E2EXAMPLE'
        return {'DistributionConfig': dict(self.config), 'ETag': 'ETAG1'}

    def update_distribution(self, Id, IfMatch, DistributionConfig):
        self.updates.append((Id, IfMatch, DistributionConfig))
        self.config = DistributionConfig

    def get_paginator(self, name):
        assert name == 'list_distributions'
        items = [{'Id': d} for d in self.distributions]

        class Pages:
            def paginate(self):
                return [{'DistributionList': {'Items': items}}, {'DistributionList': {}}]
        return Pages()


class CloudFrontControlTests(unittest.TestCase):
    def test_disable_sends_the_full_config_with_the_etag(self):
        cf = FakeCloudFront(enabled=True)
        with patch.object(guard, 'client', lambda service, region: cf):
            self.assertTrue(guard.set_distribution_enabled(False))
        self.assertEqual(cf.updates, [('E2EXAMPLE', 'ETAG1', {'CallerReference': 'x', 'Enabled': False,
                                                              'Comment': 'demo'})])

    def test_no_update_when_already_in_the_requested_state(self):
        cf = FakeCloudFront(enabled=False)
        with patch.object(guard, 'client', lambda service, region: cf):
            self.assertFalse(guard.set_distribution_enabled(False))
        self.assertEqual(cf.updates, [])

    def test_usage_sums_every_distribution_in_the_account(self):
        cf = FakeCloudFront(enabled=True, distributions=('E2EXAMPLE', 'EOTHER'))
        seen = []

        def sums(namespace, metric, start, end):
            seen.append((namespace, metric))
            return {'Requests': 1000, 'BytesDownloaded': 5 * 10 ** 6}[metric]

        cw = FakeCloudWatch(sums)
        start = guard.dt.datetime(2026, 9, 1, tzinfo=guard.dt.timezone.utc)
        now = guard.dt.datetime(2026, 9, 2, tzinfo=guard.dt.timezone.utc)
        with patch.object(guard, 'client', lambda service, region: {'cloudfront': cf, 'cloudwatch': cw}[service]):
            usage = guard.cloudfront_usage(start, now)
        self.assertEqual(usage, {'cloudFrontRequests': 2000, 'cloudFrontBytes': 10 ** 7, 'distributionCount': 2})
        self.assertEqual(set(seen), {('AWS/CloudFront', 'Requests'), ('AWS/CloudFront', 'BytesDownloaded')})


def evidence(**overrides):
    values = {'filter_ok': True, 'before': 0, 'recent': 0, 'checked_invocations': 0,
              'checked_log_events': 0, 'init_starts': 0}
    values.update(overrides)
    return values


class ColdStartEstimateTests(unittest.TestCase):
    def test_warm_health_checks_are_not_charged_a_cold_start_each(self):
        # The review's abuse case: a /healthz loop. 800,000 warm 2 ms calls at 256 MB
        # with 20 observed environment starts is far below the compute threshold.
        inits = guard.estimate_inits(800000, evidence(
            recent=10000, checked_invocations=790000, checked_log_events=2370000, init_starts=20))
        self.assertEqual(inits, 10020)
        compute = guard.compute_gb_seconds(256, 800000 * 2.0, inits)
        self.assertAlmostEqual(compute, (1600 + 10020 * 10) * 0.25)
        self.assertFalse(guard.breached({**BELOW, 'requests': 799999, 'computeEstimateGBSeconds': compute}))

    def test_old_flat_allowance_would_have_tripped_at_128k_requests(self):
        # Regression guard for F2: unmetered functions keep the conservative
        # per-invocation cap, which is why the demo itself must be metered.
        compute = guard.compute_gb_seconds(256, 128000 * 2.0, guard.estimate_inits(128000, None))
        self.assertGreaterEqual(compute, 320000)

    def test_unmetered_function_charges_every_invocation_a_cold_start(self):
        self.assertEqual(guard.estimate_inits(42, None), 42)

    def test_missing_metric_filter_falls_back_to_every_invocation(self):
        self.assertEqual(guard.estimate_inits(42, evidence(filter_ok=False, init_starts=1)), 42)

    def test_missing_log_delivery_falls_back_to_every_invocation(self):
        # Fewer log events than invocations means INIT_START lines may be missing too.
        self.assertEqual(guard.estimate_inits(42, evidence(
            checked_invocations=40, checked_log_events=39, init_starts=1)), 42)

    def test_invocations_before_the_filter_existed_are_charged_each(self):
        self.assertEqual(guard.estimate_inits(1500, evidence(
            before=1000, recent=100, checked_invocations=400,
            checked_log_events=1200, init_starts=5)), 1105)

    def test_recent_invocations_are_charged_when_init_metric_is_delayed(self):
        self.assertEqual(guard.estimate_inits(42, evidence(
            recent=42, init_starts=0)), 42)

    def test_unaccounted_invocation_sum_is_charged_conservatively(self):
        self.assertEqual(guard.estimate_inits(100, evidence(
            before=10, recent=20, checked_invocations=50,
            checked_log_events=150, init_starts=2)), 52)

    def test_any_metered_invocation_implies_at_least_one_cold_start(self):
        self.assertEqual(guard.estimate_inits(10, evidence(
            checked_invocations=10, checked_log_events=30, init_starts=0)), 1)
        self.assertEqual(guard.estimate_inits(0, evidence()), 0)

    def test_cold_start_cap_is_the_init_phase_limit(self):
        self.assertEqual(guard.compute_gb_seconds(1024, 0, 1), 10.0)
        self.assertEqual(guard.compute_gb_seconds(128, 1000, 0), 0.125)


class FakeLogs:
    def __init__(self, filters):
        self.filters = filters

    def describe_metric_filters(self, logGroupName, filterNamePrefix):
        assert logGroupName == '/aws/lambda/weather-bridge-demo'
        return {'metricFilters': [f for f in self.filters if f['filterName'].startswith(filterNamePrefix)]}


class FakeCloudWatch:
    def __init__(self, sums):
        self.sums = sums
        self.calls = []

    def get_metric_statistics(self, Namespace, MetricName, Dimensions, StartTime, EndTime, Period, Statistics):
        self.calls.append((Namespace, MetricName, StartTime, EndTime))
        return {'Datapoints': [{'Sum': self.sums(Namespace, MetricName, StartTime, EndTime)}]}


class MeteredEvidenceTests(unittest.TestCase):
    start = guard.dt.datetime(2026, 9, 1, tzinfo=guard.dt.timezone.utc)
    now = guard.dt.datetime(2026, 9, 20, 12, 7, tzinfo=guard.dt.timezone.utc)

    def init_filter(self, created):
        return {'filterName': guard.INIT_FILTER_NAME, 'filterPattern': guard.INIT_FILTER_PATTERN,
                'creationTime': int(created.timestamp() * 1000),
                'metricTransformations': [{'metricNamespace': guard.INIT_METRIC_NAMESPACE,
                                           'metricName': guard.INIT_METRIC_NAME}]}

    def test_reads_init_starts_and_log_delivery_since_the_filter_existed(self):
        created = guard.dt.datetime(2026, 9, 3, 8, 30, tzinfo=guard.dt.timezone.utc)
        metered_from = guard.dt.datetime(2026, 9, 3, 9, tzinfo=guard.dt.timezone.utc)
        cutoff = guard.dt.datetime(2026, 9, 20, 11, tzinfo=guard.dt.timezone.utc)

        def sums(namespace, metric, start, end):
            if (namespace, metric) == ('AWS/Lambda', 'Invocations'):
                return 100 if end <= metered_from else 25 if start == cutoff else 5000
            if (namespace, metric) == ('AWS/Logs', 'IncomingLogEvents'):
                return 15000
            if (namespace, metric) == (guard.INIT_METRIC_NAMESPACE, guard.INIT_METRIC_NAME):
                return 7
            raise AssertionError((namespace, metric))

        cw = FakeCloudWatch(sums)
        found = guard.metered_evidence(FakeLogs([self.init_filter(created)]), cw,
                                       'weather-bridge-demo', self.start, self.now)
        self.assertEqual(found, evidence(before=100, recent=25, checked_invocations=5000,
                                         checked_log_events=15000, init_starts=7))
        before_call = [c for c in cw.calls if c[1] == 'Invocations' and c[3] == metered_from]
        self.assertEqual(before_call, [('AWS/Lambda', 'Invocations', self.start, metered_from)])
        checked = [c for c in cw.calls if c[1] == 'IncomingLogEvents']
        self.assertEqual(checked[0][2:], (metered_from, cutoff))
        starts = [c for c in cw.calls if c[1] == guard.INIT_METRIC_NAME]
        self.assertEqual(starts[0][2:], (metered_from, cutoff))
        recent = [c for c in cw.calls if c[1] == 'Invocations' and c[2] == cutoff]
        self.assertEqual(recent, [('AWS/Lambda', 'Invocations', cutoff, self.now)])

    def test_new_filter_has_no_trusted_interval_or_overlap(self):
        created = self.now - guard.dt.timedelta(minutes=2)
        cw = FakeCloudWatch(lambda namespace, metric, s, e: 42)
        found = guard.metered_evidence(FakeLogs([self.init_filter(created)]), cw,
                                      'weather-bridge-demo', self.start, self.now)
        self.assertEqual(found, evidence(before=42))
        self.assertEqual(cw.calls, [('AWS/Lambda', 'Invocations', self.start, self.now)])
        self.assertEqual(guard.estimate_inits(42, found), 42)

    def test_month_start_before_delivery_cutoff_has_only_recent_invocations(self):
        now = self.start + guard.dt.timedelta(minutes=7)
        cw = FakeCloudWatch(lambda namespace, metric, s, e: 42)
        found = guard.metered_evidence(FakeLogs([self.init_filter(self.start)]), cw,
                                      'weather-bridge-demo', self.start, now)
        self.assertEqual(found, evidence(recent=42))
        self.assertEqual(cw.calls, [('AWS/Lambda', 'Invocations', self.start, now)])

    def test_missing_or_altered_filter_is_not_trusted(self):
        cw = FakeCloudWatch(lambda *a: 0)
        self.assertFalse(guard.metered_evidence(FakeLogs([]), cw, 'weather-bridge-demo',
                                                self.start, self.now)['filter_ok'])
        altered = self.init_filter(self.start)
        altered['filterPattern'] = '"REPORT"'
        self.assertFalse(guard.metered_evidence(FakeLogs([altered]), cw, 'weather-bridge-demo',
                                                self.start, self.now)['filter_ok'])


class FakeLambda:
    def __init__(self, functions):
        self.functions = functions

    def get_paginator(self, name):
        assert name == 'list_functions'
        functions = self.functions

        class Pages:
            def paginate(self):
                return [{'Functions': functions}]
        return Pages()


class RegionUsageTests(unittest.TestCase):
    def test_only_the_demo_in_its_region_uses_cold_start_evidence(self):
        start = guard.dt.datetime(2026, 9, 1, tzinfo=guard.dt.timezone.utc)
        now = guard.dt.datetime(2026, 9, 20, tzinfo=guard.dt.timezone.utc)
        created = int(start.timestamp() * 1000)
        logs = FakeLogs([{'filterName': guard.INIT_FILTER_NAME, 'filterPattern': guard.INIT_FILTER_PATTERN,
                          'creationTime': created,
                          'metricTransformations': [{'metricNamespace': guard.INIT_METRIC_NAMESPACE,
                                                     'metricName': guard.INIT_METRIC_NAME}]}])
        totals = {'Invocations': 1000, 'Duration': 2000, 'IncomingLogEvents': 3000,
                  guard.INIT_METRIC_NAME: 2}
        cutoff = (now - guard.LOG_DELIVERY_LAG).replace(minute=0, second=0, microsecond=0)
        def sums(namespace, metric, s, e):
            if metric == 'Invocations':
                return 10 if s == cutoff else 990 if e == cutoff else 1000
            return totals[metric]
        cw = FakeCloudWatch(sums)
        functions = [{'FunctionName': 'weather-bridge-demo', 'MemorySize': 256},
                     {'FunctionName': 'other', 'MemorySize': 128}]
        services = {'lambda': FakeLambda(functions), 'cloudwatch': cw, 'logs': logs}
        with patch.object(guard, 'client', lambda service, region: services[service]):
            requests, compute, count, demo = guard.region_usage('us-east-1', start, now)
            _, other_region_compute, _, other_demo = guard.region_usage('us-west-2', start, now)
        self.assertEqual((requests, count), (2000, 2))
        demo_compute = (2 + 12 * 10) * 0.25
        other_compute = (2 + 1000 * 10) * 0.125
        self.assertAlmostEqual(compute, demo_compute + other_compute)
        self.assertEqual(demo['chargedColdStarts'], 12)
        # A same-named function in another region is not the stack's demo.
        self.assertAlmostEqual(other_region_compute, (2 + 1000 * 10) * 0.25 + other_compute)
        self.assertIsNone(other_demo)


if __name__ == '__main__':
    unittest.main()
