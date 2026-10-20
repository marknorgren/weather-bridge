"""Recovery contracts use fake AWS boundaries, never a live account."""
import copy
import datetime as dt
import importlib.util
import json
import io
import os
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

os.environ.update(TARGET_FUNCTION='weather-bridge-demo', TARGET_REGION='us-east-1',
                  RESUME_CONCURRENCY='3', DISTRIBUTION_ID='E2EXAMPLE', ACCOUNT_ID='000000000000',
                  SCHEDULE_ARN='arn:aws:events:us-east-1:000000000000:rule/weather-bridge-usage-check',
                  STOP_TOPIC_ARN='arn:aws:sns:us-east-1:000000000000:weather-bridge-guard-events')
spec = importlib.util.spec_from_file_location('recovery_guard', Path(__file__).with_name('guard.py'))
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)
BELOW = dict(requests=1, computeEstimateGBSeconds=1, cloudFrontRequests=1, cloudFrontBytes=1)
UTC = dt.timezone.utc
START = dt.datetime(2026, 10, 19, 9, 0, tzinfo=UTC)
DEPLOYMENT_ID = 'a' * 32


class AwsError(Exception):
    def __init__(self, code='InternalFailure'):
        self.response = {'Error': {'Code': code}, 'ResponseMetadata': {'RequestId': 'aws-request'}}
        self.operation_name = 'ListFunctions'


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.boundary_print = patch('builtins.print')
        self.boundary_print.start()
        self.addCleanup(self.boundary_print.stop)
        self.now = START
        self.state = dict(version=1, mode='running', reason='operator_resume',
                          since=START.isoformat(), successes=0, lastCheck=None, signals=[])
        self.calls = []
        self.enabled = True
        self.concurrency = 3
        self.collect = self.boundary('collect_usage', return_value=BELOW)
        self.boundary('utcnow', side_effect=lambda: self.now, create=True)
        self.boundary('load_state', side_effect=lambda: copy.deepcopy(self.state), create=True)
        self.boundary('save_state', side_effect=self.save, create=True)
        self.boundary('notify', create=True)
        self.boundary('recovery_blocker', return_value=None, create=True)
        self.boundary('distribution_enabled', side_effect=lambda: self.enabled)
        self.boundary('set_distribution_enabled', side_effect=self.set_enabled)
        self.boundary('set_concurrency', side_effect=self.set_concurrency)
        sdk = SimpleNamespace(get_function_concurrency=lambda **_: {
            'ReservedConcurrentExecutions': self.concurrency})
        self.boundary('client', return_value=sdk)

    def boundary(self, name, **kwargs):
        p = patch.object(guard, name, **kwargs)
        result = p.start()
        self.addCleanup(p.stop)
        return result

    def save(self, state):
        self.state = copy.deepcopy(state)
        self.calls.append(('state', state['mode'], state['reason']))

    def set_enabled(self, enabled):
        self.calls.append(('distribution', enabled))
        self.enabled = enabled

    def set_concurrency(self, value):
        self.calls.append(('concurrency', value))
        self.concurrency = value

    def check(self, minutes):
        self.now = START + dt.timedelta(minutes=minutes)
        return guard.handler({'source': 'aws.events', 'detail-type': 'Scheduled Event',
                              'resources': [guard.SCHEDULE_ARN], 'id': 'event-' + str(minutes),
                              'time': self.now.isoformat()}, None)

    def pause(self):
        self.collect.side_effect = guard.UsageReadError('lambda', 'us-west-2', 'list_functions', AwsError())
        result = self.check(1)
        self.assertTrue(result['stopped'])
        self.assertEqual(self.state['mode'], 'paused')
        self.assertEqual(self.concurrency, 0)
        self.assertFalse(self.enabled)
        self.collect.side_effect = None
        self.calls.clear()

    def test_temporary_error_then_two_distinct_complete_checks_recovers(self):
        self.pause()
        self.check(5)
        self.assertEqual(self.concurrency, 0)
        result = self.check(10)
        self.assertFalse(result['stopped'])
        self.assertEqual(self.concurrency, 3)
        self.assertTrue(self.enabled)
        self.assertEqual(self.state['mode'], 'running')

    def test_duplicate_early_manual_and_status_checks_cannot_count_twice(self):
        self.pause()
        self.check(5)
        self.check(5)
        self.check(6)
        guard.handler({'action': 'check'}, None)
        guard.handler({'action': 'status'}, None)
        self.assertEqual(self.concurrency, 0)
        self.assertEqual(self.state['successes'], 1)

    def test_incomplete_scan_resets_recovery_evidence(self):
        self.pause()
        self.check(5)
        self.collect.side_effect = guard.UsageReadError('lambda', 'us-west-2', 'list_functions', AwsError())
        self.check(10)
        self.collect.side_effect = None
        self.check(15)
        self.assertEqual(self.concurrency, 0)
        self.check(20)
        self.assertEqual(self.concurrency, 3)

    def test_manual_stop_wins_over_later_successful_checks(self):
        self.pause()
        guard.handler({'action': 'stop'}, None)
        self.check(5)
        self.check(10)
        self.assertEqual(self.state['mode'], 'locked')
        self.assertEqual(self.state['reason'], 'manual_stop')
        self.assertEqual(self.concurrency, 0)

    def test_deployment_preserves_a_manual_lock_and_cannot_resume_it(self):
        guard.handler({'action': 'stop'}, None)
        result = guard.handler({'action': 'stop', 'deployment': True, 'deploymentId': DEPLOYMENT_ID}, None)
        self.assertFalse(result['resumeAllowed'])
        self.assertEqual(self.state['reason'], 'manual_stop')
        result = guard.handler({'action': 'resume', 'deployment': True, 'deploymentId': DEPLOYMENT_ID}, None)
        self.assertTrue(result['stopped'])
        self.assertEqual(self.concurrency, 0)

    def test_stale_deployment_intent_cannot_authorize_a_different_release(self):
        guard.handler({'action': 'stop', 'deployment': True, 'deploymentId': DEPLOYMENT_ID}, None)
        other = {'deployment': True, 'deploymentId': 'b' * 32}
        result = guard.handler({'action': 'stop', **other}, None)
        self.assertFalse(result['resumeAllowed'])
        result = guard.handler({'action': 'resume', **other}, None)
        self.assertFalse(result['resumeAllowed'])
        self.assertEqual(self.concurrency, 0)

    def test_deployment_can_resume_only_its_own_stop(self):
        result = guard.handler({'action': 'stop', 'deployment': True, 'deploymentId': DEPLOYMENT_ID}, None)
        self.assertTrue(result['resumeAllowed'])
        self.assertEqual(self.state['reason'], 'deployment_stop')
        self.assertFalse(guard.handler({'action': 'resume', 'deployment': True, 'deploymentId': DEPLOYMENT_ID}, None)['stopped'])

    def test_signal_arriving_during_deployment_prevents_deployment_resume(self):
        guard.handler({'action': 'stop', 'deployment': True, 'deploymentId': DEPLOYMENT_ID}, None)
        guard.handler(self.signal(), None)
        result = guard.handler({'action': 'resume', 'deployment': True, 'deploymentId': DEPLOYMENT_ID}, None)
        self.assertTrue(result['stopped'])
        self.assertEqual(self.state['reason'], 'budget_signal')

    def test_deployment_does_not_claim_an_unrecorded_service_stop(self):
        self.concurrency = 0
        result = guard.handler({'action': 'stop', 'deployment': True, 'deploymentId': DEPLOYMENT_ID}, None)
        self.assertFalse(result['resumeAllowed'])
        self.assertEqual(self.state['reason'], 'unrecorded_stop')

    def test_proven_absent_state_initializes_locked_without_emitting_an_error(self):
        self.boundary('load_state', side_effect=guard.MissingState('missing state'))
        result = self.check(5)
        self.assertTrue(result['stopped'])
        self.assertEqual(self.state['reason'], 'unknown_state')
        self.assertEqual(self.concurrency, 0)

    def test_documented_service_fault_and_unknown_server_errors_pause(self):
        for code, status in [('InternalServiceFault', 500), ('NewServiceFault', 503),
                             ('NewThrottle', 429), ('Unavailable', 503), ('InternalErrorException', 500)]:
            with self.subTest(code=code):
                self.state = guard.state_record('running', 'operator_resume')
                self.concurrency, self.enabled = 3, True
                error = AwsError(code)
                error.response['ResponseMetadata']['HTTPStatusCode'] = status
                self.collect.side_effect = guard.UsageReadError('cloudwatch', 'us-east-1', 'get_metric_statistics', error)
                result = self.check(1)
                self.assertEqual(result['mode'], 'paused')

    def test_persistent_fault_does_not_notify_on_every_check(self):
        self.collect.side_effect = ValueError('broken guard')
        for minute in (1, 5, 10):
            with self.assertRaises(ValueError):
                self.check(minute)
        guard.notify.assert_called_once()

    def test_unreadable_state_relies_on_alarm_instead_of_repeated_owner_messages(self):
        self.boundary('load_state', side_effect=PermissionError('state denied'))
        for minute in (1, 5):
            with self.assertRaises(PermissionError):
                self.check(minute)
        guard.notify.assert_not_called()

    def test_threshold_stop_cannot_auto_recover(self):
        self.pause()
        self.collect.return_value = {**BELOW, 'requests': 800000}
        self.check(5)
        self.collect.return_value = BELOW
        self.check(10)
        self.check(15)
        self.assertEqual(self.state['reason'], 'usage_limit')
        self.assertEqual(self.concurrency, 0)

    def test_permission_error_never_becomes_recoverable(self):
        self.collect.side_effect = guard.UsageReadError('lambda', 'us-west-2', 'list_functions',
                                                      AwsError('AccessDeniedException'))
        with self.assertRaises(guard.UsageReadError):
            self.check(1)
        self.collect.side_effect = None
        self.check(5)
        self.check(10)
        self.assertEqual(self.state['mode'], 'locked')
        self.assertEqual(self.concurrency, 0)

    def test_unknown_out_of_band_stop_is_not_recovery_eligible(self):
        self.concurrency = 0
        self.collect.side_effect = guard.UsageReadError('lambda', 'us-west-2', 'list_functions', AwsError())
        self.check(1)
        self.assertEqual(self.state['mode'], 'locked')
        self.assertEqual(self.state['reason'], 'unrecorded_stop')

    def test_resume_persists_nonrecoverable_intent_before_enabling(self):
        self.pause()
        self.check(5)
        self.check(10)
        enabling = self.calls.index(('concurrency', 3))
        self.assertIn(('state', 'locked', 'resume_in_progress'), self.calls[:enabling])

    def test_failed_enable_stays_locked_and_zeroes_concurrency(self):
        self.pause()
        self.check(5)
        def fail_enable(enabled):
            if enabled:
                raise PermissionError('update denied')
            self.enabled = False
        self.boundary('set_distribution_enabled', side_effect=fail_enable)
        with self.assertRaises(PermissionError):
            self.check(10)
        self.assertEqual(self.concurrency, 0)
        self.assertEqual(self.state['mode'], 'locked')

    def test_active_budget_or_control_alarm_prevents_recovery(self):
        for reason in ('budget_limit', 'control_alarm'):
            with self.subTest(reason=reason):
                self.setUp()
                self.pause()
                self.boundary('recovery_blocker', return_value=reason)
                self.check(5)
                self.check(10)
                self.assertEqual(self.concurrency, 0)
                self.assertEqual(self.state['reason'], reason)

    def signal(self, message_id='budget-1', topic=None, message='budget exceeded'):
        return {'Records': [{'EventSource': 'aws:sns', 'Sns': {
            'TopicArn': topic or guard.STOP_TOPIC_ARN, 'MessageId': message_id,
            'Message': message, 'Timestamp': self.now.isoformat()}}]}

    def test_budget_stop_is_latched_and_duplicate_does_not_restop_after_operator_resume(self):
        self.pause()
        event = self.signal()
        guard.handler(event, None)
        self.check(5)
        self.check(10)
        self.assertEqual(self.state['reason'], 'budget_signal')
        guard.handler({'action': 'resume'}, None)
        guard.handler(event, None)
        self.assertEqual(self.concurrency, 3)
        self.assertEqual(self.state['mode'], 'running')

    def test_foreign_topic_or_malformed_signal_is_locked(self):
        with self.assertRaises(ValueError):
            guard.handler(self.signal(topic='arn:foreign'), None)
        self.assertEqual(self.state['mode'], 'locked')
        self.assertEqual(self.concurrency, 0)

    def test_unreadable_state_stops_even_when_usage_is_below_limits(self):
        self.boundary('load_state', side_effect=PermissionError('state denied'))
        with self.assertRaises(PermissionError):
            self.check(5)
        self.assertEqual(self.concurrency, 0)

    def test_gap_in_scheduled_checks_starts_recovery_count_again(self):
        self.pause()
        self.check(5)
        self.check(20)
        self.assertEqual(self.concurrency, 0)
        self.assertEqual(self.state['successes'], 1)
        self.check(25)
        self.assertEqual(self.concurrency, 3)

    def test_future_stale_and_foreign_schedule_events_cannot_recover(self):
        self.pause()
        self.check(5)
        self.now = START + dt.timedelta(minutes=10)
        for event in [
            {'time': (START + dt.timedelta(minutes=20)).isoformat()},
            {'time': START.isoformat()},
            {'resources': ['foreign-schedule']},
        ]:
            native = {'source': 'aws.events', 'detail-type': 'Scheduled Event',
                      'resources': [guard.SCHEDULE_ARN], 'id': 'id', 'time': self.now.isoformat()}
            guard.handler({**native, **event}, None)
        self.assertEqual(self.concurrency, 0)

    def test_unexpected_error_with_temporary_code_is_not_recovery_eligible(self):
        self.collect.side_effect = AwsError()
        with self.assertRaises(AwsError):
            self.check(1)
        self.assertEqual(self.state['mode'], 'locked')

    def test_state_write_failure_still_stops_and_cannot_auto_recover(self):
        original = copy.deepcopy(self.state)
        self.boundary('save_state', side_effect=PermissionError('state write denied'))
        self.collect.side_effect = guard.UsageReadError('lambda', 'us-west-2', 'list_functions', AwsError())
        with self.assertRaises(PermissionError):
            self.check(1)
        self.assertEqual(self.concurrency, 0)
        self.assertEqual(self.state, original)

    def test_crash_during_resume_leaves_a_locked_record(self):
        self.pause()
        self.check(5)
        def crash(enabled):
            if enabled:
                raise SystemExit('simulated invocation termination')
            self.enabled = False
        self.boundary('set_distribution_enabled', side_effect=crash)
        with self.assertRaises(SystemExit):
            self.check(10)
        self.assertEqual(self.state['mode'], 'locked')
        self.assertEqual(self.state['reason'], 'resume_in_progress')
        self.check(15)
        self.assertEqual(self.concurrency, 0)

    def test_missing_or_malformed_state_requires_explicit_resume(self):
        self.boundary('load_state', side_effect=guard.StateError('missing state'))
        with self.assertRaises(guard.StateError):
            self.check(5)
        self.assertEqual(self.concurrency, 0)
        result = guard.handler({'action': 'resume'}, None)
        self.assertFalse(result['stopped'])

    def test_manual_stop_records_intent_even_when_previous_state_is_missing(self):
        self.boundary('load_state', side_effect=guard.StateError('missing state'))
        result = guard.handler({'action': 'stop'}, None)
        self.assertTrue(result['stopped'])
        self.assertEqual(self.state['reason'], 'manual_stop')
        self.assertEqual(self.concurrency, 0)

    def test_locked_stop_survives_temporary_usage_failure(self):
        guard.handler({'action': 'stop'}, None)
        self.collect.side_effect = guard.UsageReadError('lambda', 'us-west-2', 'list_functions', AwsError())
        self.check(1)
        self.assertEqual(self.state['mode'], 'locked')
        self.assertEqual(self.state['reason'], 'manual_stop')


class BoundaryTests(unittest.TestCase):
    def test_state_round_trip_uses_only_the_fixed_encrypted_private_object(self):
        class Storage:
            def list_objects_v2(self, **kw):
                self.listing = kw
                return {'Contents': [{'Key': guard.STATE_KEY}]}
            def get_object(self, **kw):
                self.get = kw
                self.body = io.BytesIO(self.value)
                return {'Body': self.body}
            def put_object(self, **kw):
                self.put = kw
                self.value = kw['Body']
        storage = Storage()
        with patch.object(guard, 'client', return_value=storage), patch.object(guard, 'STATE_BUCKET', 'private-bucket'):
            state = guard.state_record('locked', 'manual_stop')
            guard.save_state(state)
            self.assertEqual(guard.load_state(), state)
        self.assertEqual(storage.get, {'Bucket': 'private-bucket', 'Key': 'guard-state/state.json'})
        self.assertEqual(storage.listing['Prefix'], 'guard-state/state.json')
        self.assertEqual(storage.put['ServerSideEncryption'], 'AES256')
        self.assertTrue(storage.body.closed)

    def test_missing_state_does_not_get_a_different_prefix_object(self):
        storage = SimpleNamespace(list_objects_v2=lambda **_: {'Contents': [{'Key': guard.STATE_KEY + '.bak'}]})
        with patch.object(guard, 'client', return_value=storage), self.assertRaises(guard.StateError):
            guard.load_state()

    def test_state_rejects_unknown_eligibility_and_inconsistent_success_evidence(self):
        state = guard.state_record('paused', 'monitoring_failure')
        for change in ({'reason': 'manual_stop'}, {'mode': 'unknown'}, {'version': True},
                       {'successes': 2}, {'successes': 1}, {'signals': ['x'] * 17},
                       {'successes': 1, 'lastCheck': '1970-01-01T00:00:00+00:00'}):
            with self.subTest(change=change), self.assertRaises(guard.StateError):
                guard.validate_state({**state, **change})

    def test_malformed_and_oversized_state_reads_close_the_body(self):
        for data in (b'{}', b'[]', b'not json', b'x' * 5000):
            body = io.BytesIO(data)
            storage = SimpleNamespace(
                list_objects_v2=lambda **_: {'Contents': [{'Key': guard.STATE_KEY}]},
                get_object=lambda **_: {'Body': body})
            with patch.object(guard, 'client', return_value=storage), self.assertRaises(guard.StateError):
                guard.load_state()
            self.assertTrue(body.closed)

    def test_usage_errors_include_context_and_do_not_log_raw_messages(self):
        error = AwsError()
        sdk = SimpleNamespace(meta=SimpleNamespace(region_name='us-west-2',
                              service_model=SimpleNamespace(service_name='lambda')))
        def fail():
            raise error
        sdk.list_functions = fail
        with self.assertRaises(guard.UsageReadError) as caught:
            guard.usage_read(sdk, 'list_functions')
        self.assertTrue(caught.exception.temporary)
        self.assertEqual(caught.exception.details, {'service': 'lambda', 'region': 'us-west-2',
                         'operation': 'list_functions', 'code': 'InternalFailure', 'awsRequestId': 'aws-request'})
        self.assertTrue(caught.exception.__suppress_context__)

    def test_list_functions_pagination_failure_keeps_region_and_operation(self):
        def fail():
            raise AwsError()
            yield  # Model an SDK paginator that fails when the first page is read.
        sdk = SimpleNamespace(meta=SimpleNamespace(region_name='us-west-2',
                              service_model=SimpleNamespace(service_name='lambda')),
                              get_paginator=lambda _: SimpleNamespace(paginate=fail))
        with self.assertRaises(guard.UsageReadError) as caught:
            list(guard.usage_pages(sdk, 'list_functions'))
        self.assertEqual(caught.exception.details['region'], 'us-west-2')
        self.assertEqual(caught.exception.details['operation'], 'list_functions')
        self.assertTrue(caught.exception.__suppress_context__)

    def test_one_failed_region_prevents_partial_account_totals_from_being_returned(self):
        sdk = SimpleNamespace(describe_regions=lambda: {'Regions': [
            {'RegionName': 'us-east-1'}, {'RegionName': 'us-west-2'}]})
        def region_usage(region, *_):
            if region == 'us-west-2':
                raise guard.UsageReadError('lambda', region, 'list_functions', AwsError())
            return 1, 1, 1, None
        with patch.object(guard, 'client', return_value=sdk), patch.object(
                guard, 'region_usage', side_effect=region_usage), patch.object(guard, 'cloudfront_usage') as cf:
            with self.assertRaises(guard.UsageReadError):
                guard.collect_usage()
            cf.assert_not_called()

    def test_notification_failure_is_observable_without_raising(self):
        def fail(**_):
            raise PermissionError('denied')
        output = io.StringIO()
        with patch.object(guard, 'OWNER_TOPIC_ARN', 'owner-topic'), patch.object(
                guard, 'client', return_value=SimpleNamespace(publish=fail)), patch('sys.stdout', output):
            guard.notify('recovered', guard.state_record('running', 'monitoring_recovered'))
        self.assertIn('notification_failed', output.getvalue())

    def test_recovery_requires_complete_ok_alarms_and_below_budget(self):
        alarms = [{'AlarmName': name, 'StateValue': 'OK'} for name in guard.CONTROL_ALARMS]
        actual = {'Amount': '0.009', 'Unit': 'USD'}
        sdk = SimpleNamespace(
            describe_alarms=lambda **_: {'MetricAlarms': alarms},
            describe_budget=lambda **_: {'Budget': {'CalculatedSpend': {'ActualSpend': actual},
                                                     'BudgetLimit': {'Amount': '1', 'Unit': 'USD'}}})
        with patch.object(guard, 'client', return_value=sdk):
            self.assertIsNone(guard.recovery_blocker())
            actual['Amount'] = '0.010'
            self.assertIsNone(guard.recovery_blocker())
            actual['Amount'] = '0.011'
            self.assertEqual(guard.recovery_blocker(), 'budget_limit')
            actual['Amount'] = 'NaN'
            with self.assertRaises(ValueError):
                guard.recovery_blocker()
            alarms.pop()
            self.assertEqual(guard.recovery_blocker(), 'control_alarm')

    def test_missing_calculated_spend_refuses_resume_without_a_guard_exception(self):
        sdk = SimpleNamespace(
            describe_alarms=lambda **_: {'MetricAlarms': [
                {'AlarmName': name, 'StateValue': 'OK'} for name in guard.CONTROL_ALARMS]},
            describe_budget=lambda **_: {'Budget': {'BudgetLimit': {'Amount': '1', 'Unit': 'USD'}}})
        with patch.object(guard, 'client', return_value=sdk):
            self.assertEqual(guard.recovery_blocker(), 'budget_unknown')


if __name__ == '__main__':
    unittest.main()
