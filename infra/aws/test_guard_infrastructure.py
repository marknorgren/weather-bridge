"""Check the actual template boundary, including references and ownership."""
import json
from pathlib import Path
import re
import unittest


TEMPLATE = json.loads(Path(__file__).with_name('template.json').read_text())
RESOURCES = TEMPLATE['Resources']


def references(value):
    if isinstance(value, dict):
        if 'Ref' in value:
            yield value['Ref']
        if 'Fn::GetAtt' in value:
            yield value['Fn::GetAtt'][0]
        if 'Fn::Sub' in value:
            sub = value['Fn::Sub']
            if isinstance(sub, str):
                yield from (v.split('.')[0] for v in re.findall(r'\$\{([^}]+)\}', sub))
        for child in value.values():
            yield from references(child)
    elif isinstance(value, list):
        for child in value:
            yield from references(child)


class GuardInfrastructureTests(unittest.TestCase):
    def test_schedule_delivers_native_identity_and_time_to_a_single_writer(self):
        schedule = RESOURCES['GuardSchedule']['Properties']
        self.assertEqual(schedule['Name'], 'weather-bridge-usage-check')
        self.assertEqual(schedule['ScheduleExpression'], 'rate(5 minutes)')
        self.assertNotIn('Input', schedule['Targets'][0])
        self.assertEqual(RESOURCES['GuardFunction']['Properties']['ReservedConcurrentExecutions'], 1)
        variables = RESOURCES['GuardFunction']['Properties']['Environment']['Variables']
        self.assertIn('weather-bridge-usage-check', variables['SCHEDULE_ARN']['Fn::Sub'])

    def test_state_permissions_allow_only_one_object_and_prefix(self):
        statements = RESOURCES['GuardRole']['Properties']['Policies'][0]['PolicyDocument']['Statement']
        storage = [s for s in statements if any(a.startswith('s3:') for a in s.get('Action', []))]
        self.assertEqual(len(storage), 2)
        write = next(s for s in storage if 's3:PutObject' in s['Action'])
        self.assertEqual(set(write['Action']), {'s3:GetObject', 's3:PutObject'})
        self.assertTrue(write['Resource']['Fn::Sub'].endswith('/guard-state/state.json'))
        listing = next(s for s in storage if 's3:ListBucket' in s['Action'])
        self.assertEqual(listing['Condition']['StringEquals']['s3:prefix'], 'guard-state/state.json')

    def test_owner_topic_cannot_invoke_the_shutdown_handler(self):
        self.assertEqual(RESOURCES['OwnerEmail']['Condition'], 'HasOwnerEmail')
        self.assertEqual(RESOURCES['OwnerEmail']['Properties']['TopicArn'], {'Ref': 'OwnerTopic'})
        for resource in RESOURCES.values():
            if resource['Type'] == 'AWS::SNS::Subscription' and resource['Properties']['Protocol'] == 'lambda':
                self.assertNotEqual(resource['Properties']['TopicArn'], {'Ref': 'OwnerTopic'})
        self.assertEqual(TEMPLATE['Parameters']['OwnerNotificationEmail']['Default'], '')

    def test_owner_address_is_masked_in_stack_and_plan_output(self):
        self.assertTrue(TEMPLATE['Parameters']['OwnerNotificationEmail']['NoEcho'])
        root = Path(__file__).resolve().parents[2]
        variables = (root / 'infra/aws/terraform/variables.tf').read_text()
        email = variables.split('variable "owner_notification_email" {')[1].split('variable "region"')[0]
        self.assertRegex(email, r'sensitive\s*=\s*true')
        workflow = (root / '.github/workflows/release.yml').read_text()
        self.assertIn('secrets.OWNER_NOTIFICATION_EMAIL', workflow)

    def test_alarm_names_match_runtime_checks_and_preserve_shutdown_backstop(self):
        for logical, name in [('GuardErrorAlarm', 'weather-bridge-guard-error'),
                              ('ScheduleFailureAlarm', 'weather-bridge-schedule-failure')]:
            props = RESOURCES[logical]['Properties']
            self.assertEqual(props['AlarmName'], name)
            self.assertIn({'Ref': 'GuardTopic'}, props['AlarmActions'])
            self.assertIn({'Ref': 'OwnerTopic'}, props['AlarmActions'])

    def test_cloudformation_resource_dependencies_have_no_cycles(self):
        graph = {name: set(references(resource)) & RESOURCES.keys() for name, resource in RESOURCES.items()}
        for name, resource in RESOURCES.items():
            depends = resource.get('DependsOn', [])
            graph[name].update([depends] if isinstance(depends, str) else depends)
        done = set()
        def visit(name, path):
            self.assertNotIn(name, path, 'dependency cycle: ' + ' -> '.join([*path, name]))
            if name not in done:
                for child in graph[name]:
                    visit(child, [*path, name])
                done.add(name)
        for name in graph:
            visit(name, [])


if __name__ == '__main__':
    unittest.main()
